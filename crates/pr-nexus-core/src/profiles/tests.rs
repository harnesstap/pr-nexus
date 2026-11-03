use super::*;
use crate::domain::{Account, AccountId, AccountKind, AuthMode, ProjectRule, RuleKind};
use crate::store::Store;

fn now() -> DateTime<Utc> {
    "2026-01-02T03:04:05Z".parse().unwrap()
}

fn account(kind: AccountKind, external_id: &str, workspace: Option<&str>, key: &str) -> Account {
    Account {
        id: AccountId(0),
        kind,
        display_name: external_id.into(),
        host: "github.com".into(),
        external_id: external_id.into(),
        slack_workspace: workspace.map(str::to_owned),
        login: "alice".into(),
        auth_mode: AuthMode::Pat,
        keychain_key: Some(key.into()),
        created_at: now(),
    }
}

#[test]
fn first_run_creates_the_default_profile_and_one_backup() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = ProfileDirs::new(dir.path().join("pr-nexus.db"));
    {
        let store = Store::open(&dirs.default_db).unwrap();
        store.create_project("Checkout".into()).unwrap();
    }
    let registry = dirs.migrate(now()).unwrap();
    assert_eq!(registry.active, DEFAULT_ID);
    assert_eq!(registry.profiles.len(), 1);
    assert_eq!(registry.profiles[0].name, DEFAULT_NAME);
    assert_eq!(dirs.database(DEFAULT_ID), dirs.default_db);
    // The backup is a full copy and the original is untouched.
    let backup = Store::open(dirs.backup_path()).unwrap();
    assert_eq!(backup.list_projects().unwrap()[0].name, "Checkout");
    let original = Store::open(&dirs.default_db).unwrap();
    assert_eq!(original.list_projects().unwrap()[0].name, "Checkout");
}

#[test]
fn migration_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = ProfileDirs::new(dir.path().join("pr-nexus.db"));
    drop(Store::open(&dirs.default_db).unwrap());
    let mut first = dirs.migrate(now()).unwrap();
    let backup = std::fs::metadata(dirs.backup_path())
        .unwrap()
        .modified()
        .unwrap();
    first.profiles.push(Profile {
        id: "work-1".into(),
        name: "Work".into(),
        created_at: now().to_rfc3339(),
    });
    first.active = "work-1".into();
    dirs.save(&first).unwrap();
    let again = dirs.migrate(now()).unwrap();
    assert_eq!(again, first, "a second run keeps the registry as saved");
    let after = std::fs::metadata(dirs.backup_path())
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(backup, after, "the backup is never retaken");
}

#[test]
fn a_fresh_install_has_no_backup() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = ProfileDirs::new(dir.path().join("pr-nexus.db"));
    dirs.migrate(now()).unwrap();
    assert!(!dirs.backup_path().exists());
}

#[test]
fn profiles_have_separate_databases() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = ProfileDirs::new(dir.path().join("pr-nexus.db"));
    let a = Store::open(dirs.database(DEFAULT_ID)).unwrap();
    let b = Store::open(dirs.database("work-1")).unwrap();
    a.create_project("Only in default".into()).unwrap();
    assert!(b.list_projects().unwrap().is_empty());
    assert_ne!(dirs.database(DEFAULT_ID), dirs.database("work-1"));
}

#[test]
fn names_are_trimmed_unique_and_bounded() {
    let taken = [Profile {
        id: "default".into(),
        name: "Default".into(),
        created_at: now().to_rfc3339(),
    }];
    assert_eq!(validate_name("  Work ", &taken).unwrap(), "Work");
    assert!(validate_name("default", &taken).is_err());
    assert!(validate_name("   ", &taken).is_err());
    assert!(validate_name(&"x".repeat(MAX_NAME + 1), &taken).is_err());
}

#[test]
fn ids_are_slugs_and_never_default() {
    let id = new_id("Acme Labs!", &[], now());
    assert!(id.starts_with("acme-labs-"), "{id}");
    assert!(new_id("Default", &[], now()).starts_with("profile-"));
    let taken = [id.as_str()];
    assert_ne!(new_id("Acme Labs!", &taken, now()), id);
}

#[test]
fn resolve_accepts_an_id_or_a_name() {
    let registry = Registry {
        version: 1,
        active: "default".into(),
        profiles: vec![
            default_profile(now()),
            Profile {
                id: "acme-1".into(),
                name: "Acme Labs".into(),
                created_at: now().to_rfc3339(),
            },
        ],
        pending_removal: vec![],
    };
    assert_eq!(registry.resolve("acme-1").unwrap().name, "Acme Labs");
    assert_eq!(registry.resolve("acme labs").unwrap().id, "acme-1");
    assert!(registry.resolve("nope").is_none());
}

#[test]
fn copy_copies_only_the_chosen_items_and_does_not_link_them() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("a.db");
    let target_path = dir.path().join("b.db");
    let source = Store::open(&source_path).unwrap();
    source
        .insert_account(account(AccountKind::GitHub, "1", None, "github:alice"))
        .unwrap();
    source
        .insert_account(account(
            AccountKind::Slack,
            "T1:U1",
            Some("acme"),
            "slack:T1:U1",
        ))
        .unwrap();
    source
        .insert_account(account(
            AccountKind::Slack,
            "T2:U2",
            Some("acme-labs"),
            "slack:T2:U2",
        ))
        .unwrap();
    let project = source.create_project("Checkout".into()).unwrap();
    source
        .insert_project_rule(ProjectRule {
            id: 0,
            project: project.id,
            kind: RuleKind::Label,
            pattern: "checkout".into(),
            priority: 0,
        })
        .unwrap();
    crate::settings::set_tracked_orgs(&source, &["acme".into()]).unwrap();
    crate::settings::set_message_template(
        &source,
        crate::review::message_template::TemplateKind::SingleRequest,
        Some("Please review {link}"),
    )
    .ok();
    let target = Store::open(&target_path).unwrap();
    drop(target);

    let keys = copy_items(
        &target_path,
        &source_path,
        &ReuseItems {
            github: true,
            slack_teams: vec!["T2".into()],
            projects: true,
            ..ReuseItems::default()
        },
    )
    .unwrap();
    assert_eq!(
        keys,
        vec!["github:alice".to_owned(), "slack:T2:U2".to_owned()]
    );

    let target = Store::open(&target_path).unwrap();
    let accounts = target.list_accounts().unwrap();
    let ids: Vec<_> = accounts.iter().map(|a| a.external_id.as_str()).collect();
    assert_eq!(ids, ["1", "T2:U2"]);
    assert_eq!(crate::settings::tracked_orgs(&target).unwrap(), ["acme"]);
    let copied = target.list_projects().unwrap();
    assert_eq!(copied.len(), 1);
    assert_eq!(target.list_project_rules(copied[0].id).unwrap().len(), 1);
    // Templates were not picked.
    assert!(
        crate::settings::message_templates(&target).unwrap()
            == crate::review::message_template::MessageTemplates::default()
    );

    // A copy, not a link: renaming in the target leaves the source alone.
    target
        .rename_project(copied[0].id, "Payments".into())
        .unwrap();
    assert_eq!(source.list_projects().unwrap()[0].name, "Checkout");

    // Copying again adds no duplicates.
    drop(target);
    copy_items(
        &target_path,
        &source_path,
        &ReuseItems {
            github: true,
            ..ReuseItems::default()
        },
    )
    .unwrap();
    let target = Store::open(&target_path).unwrap();
    assert_eq!(target.list_accounts().unwrap().len(), 2);
}
