//! The optional `profile` argument of every tool (§8.4): tools act on the
//! active profile unless an agent names another one.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use rmcp::model::JsonObject;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::AppError;
use crate::state::AppState;

/// The argument every tool accepts besides its own.
pub const ARG: &str = "profile";

/// States for profiles other than the active one, keyed on the manager and
/// the profile id. Each has the profile's store and vault scope.
static OTHERS: LazyLock<Mutex<HashMap<String, Arc<AppState>>>> = LazyLock::new(Default::default);

fn cache_key(state: &AppState, id: &str) -> String {
    format!("{:p}/{id}", Arc::as_ptr(&state.profiles))
}

/// Takes `profile` out of `arguments`, so each tool's own argument check
/// never sees it.
pub fn take(arguments: &mut JsonObject) -> Result<Option<String>, AppError> {
    match arguments.remove(ARG) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(key)) if !key.trim().is_empty() => Ok(Some(key)),
        Some(_) => Err(AppError::new(
            "validation",
            "`profile` must be a profile name or id; see list_profiles",
        )),
    }
}

/// The state a call runs against: the active one, or the named profile's.
pub fn resolve(main: &Arc<AppState>, key: Option<&str>) -> Result<Arc<AppState>, AppError> {
    let Some(key) = key else {
        return Ok(Arc::clone(main));
    };
    let profile = main.profiles.resolve(key)?;
    if profile.id == main.profile() {
        return Ok(Arc::clone(main));
    }
    let mut others = OTHERS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(state) = others.get(&cache_key(main, &profile.id)) {
        return Ok(Arc::clone(state));
    }
    let state = Arc::new(AppState::build(
        Arc::clone(&main.profiles),
        &profile.id,
        Arc::new(main.vault.view(&profile.id)),
        main.demo,
    )?);
    others.insert(cache_key(main, &profile.id), Arc::clone(&state));
    Ok(state)
}

/// Drops the cached state of `id` (it became active, or was deleted).
pub fn forget(id: &str) {
    let suffix = format!("/{id}");
    OTHERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|key, _| !key.ends_with(&suffix));
}

/// "Profile: Work" for confirmations, once there is more than one profile.
pub fn label(state: &AppState) -> Option<String> {
    let registry = state.profiles.registry();
    if registry.profiles.len() < 2 {
        return None;
    }
    registry.get(&state.profile()).map(|p| p.name.clone())
}

/// Adds the `profile` property to a tool's input schema.
pub fn with_arg(schema: Arc<JsonObject>) -> Arc<JsonObject> {
    let mut schema = (*schema).clone();
    let properties = schema
        .entry("properties")
        .or_insert_with(|| Value::Object(Default::default()));
    if let Value::Object(properties) = properties {
        properties.insert(
            ARG.into(),
            json!({
                "type": "string",
                "description": "Profile name or id to act on (see list_profiles). Defaults to the active profile."
            }),
        );
    }
    Arc::new(schema)
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ListProfilesArgs {}

/// `list_profiles`: every profile, the active one flagged.
pub fn list_profiles(state: &AppState, arguments: JsonObject) -> Result<Value, AppError> {
    let _: ListProfilesArgs = super::tools::parse(arguments)?;
    let registry = state.profiles.registry();
    let profiles: Vec<Value> = registry
        .profiles
        .iter()
        .map(|p| json!({ "id": p.id, "name": p.name, "active": p.id == registry.active }))
        .collect();
    Ok(json!({ "profiles": profiles, "active": registry.active }))
}
