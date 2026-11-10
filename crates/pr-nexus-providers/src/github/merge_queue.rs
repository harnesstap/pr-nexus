//! Merge queue reads and the enqueue / dequeue mutations.
//!
//! `BranchProtectionRule` has no `requiresMergeQueue` field and `Ref` has no
//! `mergeQueue` field. The pull request fields below are the ones GitHub
//! actually returns: `isMergeQueueEnabled` (branch protection "Require merge
//! queue", and a ruleset rule of type `MERGE_QUEUE`), `mergeQueue` (the base
//! branch's queue), and `mergeQueueEntry`.

use pr_nexus_core::domain::PullRequest;
use pr_nexus_core::error::ProviderError;
use pr_nexus_core::merge_queue::{
    MergeQueueReading, MergeQueueSnapshot, QueueEvent, map_merge_queue,
};
use serde_json::{Value, json};

use super::GithubClient;
use super::overview::permission_error;
use super::parse::data;

const ENQUEUE: &str = include_str!("graphql/enqueue_pull_request.graphql");
const DEQUEUE: &str = include_str!("graphql/dequeue_pull_request.graphql");

pub(crate) fn snapshot_from_node(node: &Value) -> Option<MergeQueueSnapshot> {
    map_merge_queue(&reading_from_node(node))
}

pub(crate) fn reading_from_node(node: &Value) -> MergeQueueReading {
    let entry = node
        .get("mergeQueueEntry")
        .filter(|value| value.is_object());
    MergeQueueReading {
        enabled: node.get("isMergeQueueEnabled").and_then(Value::as_bool),
        has_queue: node.get("mergeQueue").is_some_and(Value::is_object),
        in_queue: node.get("isInMergeQueue").and_then(Value::as_bool) == Some(true),
        entry_state: entry
            .and_then(|entry| entry.get("state"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        position: entry
            .and_then(|entry| entry.get("position"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        eta_seconds: entry
            .and_then(|entry| entry.get("estimatedTimeToMerge"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        latest_event: latest_event(node),
    }
}

fn latest_event(node: &Value) -> Option<QueueEvent> {
    let nodes = node.pointer("/timelineItems/nodes")?.as_array()?;
    let last = nodes.iter().rev().find(|item| {
        matches!(
            item.get("__typename").and_then(Value::as_str),
            Some("AddedToMergeQueueEvent" | "RemovedFromMergeQueueEvent")
        )
    })?;
    match last.get("__typename").and_then(Value::as_str) {
        Some("RemovedFromMergeQueueEvent") => Some(QueueEvent::Removed {
            reason: last
                .get("reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
                .map(str::to_owned),
        }),
        Some("AddedToMergeQueueEvent") => Some(QueueEvent::Added),
        _ => None,
    }
}

pub(crate) fn enqueue(
    client: &GithubClient,
    pr: &PullRequest,
    expected_head_oid: &str,
) -> Result<MergeQueueSnapshot, ProviderError> {
    let body = match client.graphql(
        ENQUEUE,
        json!({ "id": pr.remote.id, "expectedHeadOid": expected_head_oid }),
    ) {
        Ok(body) => body,
        Err(err) => return Err(permission_error(err, "add to the merge queue")),
    };
    let entry = data(&body)?
        .pointer("/enqueuePullRequest/mergeQueueEntry")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(snapshot_from_entry(&entry))
}

pub(crate) fn dequeue(client: &GithubClient, pr: &PullRequest) -> Result<(), ProviderError> {
    let body = match client.graphql(DEQUEUE, json!({ "id": pr.remote.id })) {
        Ok(body) => body,
        Err(err) => return Err(permission_error(err, "remove from the merge queue")),
    };
    data(&body)?;
    Ok(())
}

fn snapshot_from_entry(entry: &Value) -> MergeQueueSnapshot {
    let mut node = serde_json::Map::new();
    node.insert("isMergeQueueEnabled".into(), Value::Bool(true));
    node.insert("isInMergeQueue".into(), Value::Bool(true));
    node.insert("mergeQueue".into(), json!({ "id": "queue" }));
    node.insert("mergeQueueEntry".into(), entry.clone());
    snapshot_from_node(&Value::Object(node)).unwrap_or(MergeQueueSnapshot {
        required: true,
        phase: Some(pr_nexus_core::merge_queue::MergeQueuePhase::Queued),
        position: None,
        eta_seconds: None,
        reason: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pr_nexus_core::merge_queue::MergeQueuePhase;

    fn node(name: &str) -> Value {
        let text = match name {
            "required" => include_str!("fixtures/merge_queue/acme_required.json"),
            "queued" => include_str!("fixtures/merge_queue/acme_queued.json"),
            "merging" => include_str!("fixtures/merge_queue/acme_merging.json"),
            "failed" => include_str!("fixtures/merge_queue/acme_failed.json"),
            "removed" => include_str!("fixtures/merge_queue/acme_removed.json"),
            "absent" => include_str!("fixtures/merge_queue/acme_absent.json"),
            "off" => include_str!("fixtures/merge_queue/acme_queue_off.json"),
            other => panic!("unknown fixture {other}"),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn acme_fixtures_map_queue_state() {
        let required = snapshot_from_node(&node("required")).unwrap();
        assert!(required.required);
        assert!(required.phase.is_none());

        let queued = snapshot_from_node(&node("queued")).unwrap();
        assert_eq!(queued.phase, Some(MergeQueuePhase::Queued));
        assert_eq!(queued.position, Some(2));
        assert_eq!(queued.eta_seconds, Some(300));

        let merging = snapshot_from_node(&node("merging")).unwrap();
        assert_eq!(merging.phase, Some(MergeQueuePhase::Merging));
        assert_eq!(merging.position, Some(1));
        assert_eq!(merging.eta_seconds, None);

        let failed = snapshot_from_node(&node("failed")).unwrap();
        assert_eq!(failed.phase, Some(MergeQueuePhase::Failed));

        let removed = snapshot_from_node(&node("removed")).unwrap();
        assert_eq!(removed.phase, Some(MergeQueuePhase::Removed));
        assert_eq!(
            removed.reason.as_deref(),
            Some("Required status check build failed.")
        );

        assert!(snapshot_from_node(&node("absent")).is_none());

        let off = snapshot_from_node(&node("off")).unwrap();
        assert!(!off.required);
    }

    #[test]
    fn a_ruleset_merge_queue_rule_is_the_enabled_flag_not_a_missing_field() {
        // Repository ruleset on acme/widgets. Rule type MERGE_QUEUE means
        // merges must use the queue. GitHub reports that on the pull request
        // as isMergeQueueEnabled. BranchProtectionRule has no
        // requiresMergeQueue field, so this protection object is not queried.
        let ruleset: Value =
            serde_json::from_str(include_str!("fixtures/merge_queue/acme_ruleset.json")).unwrap();
        let types: Vec<_> = ruleset["rules"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|rule| rule["type"].as_str())
            .collect();
        assert!(types.contains(&"MERGE_QUEUE"));
        assert!(
            ruleset["branchProtection"]
                .get("requiresMergeQueue")
                .is_none()
        );
        let queued = snapshot_from_node(&node("queued")).unwrap();
        assert!(queued.required);
    }
}
