//! Tauri commands for the account menu, the wizard and Settings.

use std::sync::Arc;

use pr_nexus_core::domain::{AccountKind, AuthMode};
use pr_nexus_core::profiles::DEFAULT_ID;
use serde::Serialize;
use tauri::State;

use super::ReuseFrom;
use crate::blocking;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDto {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub github_login: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackTeamDto {
    pub team: String,
    pub name: String,
}

/// What one profile can share with a new one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReuseOptions {
    pub profile: String,
    pub github_login: Option<String>,
    pub slack_teams: Vec<SlackTeamDto>,
    pub templates: usize,
    pub projects: usize,
    pub send_policy: bool,
}

pub fn list(state: &AppState) -> Result<Vec<ProfileDto>, AppError> {
    let registry = state.profiles.registry();
    registry
        .profiles
        .iter()
        .map(|profile| {
            let store = state.profiles.store(&profile.id)?;
            Ok(ProfileDto {
                id: profile.id.clone(),
                name: profile.name.clone(),
                active: profile.id == registry.active,
                github_login: crate::state::github_account(store)?.map(|a| a.login),
                is_default: profile.id == DEFAULT_ID,
            })
        })
        .collect()
}

pub fn reuse_options(state: &AppState, id: &str) -> Result<ReuseOptions, AppError> {
    let profile = state.profiles.resolve(id)?;
    let store = state.profiles.store(&profile.id)?;
    let accounts = store.list_accounts()?;
    let mut slack_teams: Vec<SlackTeamDto> = Vec::new();
    for account in accounts.iter().filter(|a| a.kind == AccountKind::Slack) {
        let team = account
            .external_id
            .split(':')
            .next()
            .unwrap_or_default()
            .to_owned();
        if !team.is_empty() && !slack_teams.iter().any(|t| t.team == team) {
            slack_teams.push(SlackTeamDto {
                team,
                name: account
                    .slack_workspace
                    .clone()
                    .unwrap_or_else(|| account.display_name.clone()),
            });
        }
    }
    let templates = pr_nexus_core::review::message_template::TemplateKind::ALL
        .iter()
        .filter(|kind| {
            store
                .get_setting(kind.setting_key().to_owned())
                .ok()
                .flatten()
                .is_some()
        })
        .count();
    let send_policy = store
        .get_setting(pr_nexus_core::profiles::SEND_POLICY_KEY.to_owned())?
        .is_some()
        || profile.id == DEFAULT_ID;
    Ok(ReuseOptions {
        profile: profile.id,
        github_login: crate::state::github_account(store)?.map(|a| a.login),
        slack_teams,
        templates,
        projects: store.list_projects()?.len(),
        send_policy,
    })
}

/// Creates a profile from `reuse`, copies the secrets it needs, and
/// makes it active so the wizard continues in it.
pub fn create(state: &AppState, name: &str, reuse: &[ReuseFrom]) -> Result<ProfileDto, AppError> {
    create_with(
        state,
        name,
        reuse,
        &|login| match pr_nexus_providers::github::read_gh_token_for(Some(login)) {
            pr_nexus_providers::github::GhProbe::Ready { token } => Ok(token),
            pr_nexus_providers::github::GhProbe::Unavailable { reason } => {
                Err(AppError::new("unauthorized", reason))
            }
        },
    )
}

/// Reads the token of one gh account (`gh auth token --user <login>`).
pub type GhTokenFor<'a> = &'a dyn Fn(&str) -> Result<String, AppError>;

/// [`create`] with the gh token reader passed in, for tests.
///
/// A GitHub account copied from a profile that follows gh's active
/// account (only the default one does) gets a fixed token in the new
/// profile: the token gh holds for that login now, saved under the new
/// profile's keys. Later `gh` account changes never move the new profile
/// to another account (§8.4).
pub fn create_with(
    state: &AppState,
    name: &str,
    reuse: &[ReuseFrom],
    gh_token_for: GhTokenFor<'_>,
) -> Result<ProfileDto, AppError> {
    // Read the token before anything is created, so a gh failure leaves
    // no half-made profile behind.
    let mut pinned = None;
    for from in reuse.iter().filter(|from| from.items.github) {
        let source = state.profiles.resolve(&from.profile)?;
        let store = state.profiles.store(&source.id)?;
        if let Some(account) = crate::state::github_account(store)?
            .filter(|account| account.auth_mode == AuthMode::GhCli)
        {
            let token = gh_token_for(&account.login)?;
            pinned = Some((account.external_id, token));
        }
    }
    let (profile, secrets) = state.profiles.create(name, reuse)?;
    for (source, keys) in secrets {
        state.vault.copy_between(&source, &profile.id, &keys)?;
    }
    state.switch_profile(&profile.id)?;
    if let Some((external_id, token)) = pinned {
        let account = state
            .store
            .list_accounts()?
            .into_iter()
            .find(|a| a.kind == AccountKind::GitHub && a.external_id == external_id);
        if let Some(mut account) = account {
            let key = format!("github:{external_id}");
            state.vault.set(&key, &token)?;
            account.auth_mode = AuthMode::Pat;
            account.keychain_key = Some(key);
            state.store.upsert_account(account)?;
        }
    }
    Ok(ProfileDto {
        id: profile.id.clone(),
        name: profile.name,
        active: true,
        github_login: crate::state::github_account(&state.store)?.map(|a| a.login),
        is_default: false,
    })
}

pub fn delete(state: &AppState, id: &str) -> Result<(), AppError> {
    let profile = state.profiles.resolve(id)?;
    state.profiles.delete(&profile.id)?;
    state.vault.remove_profile(&profile.id)?;
    crate::mcp::profiles::forget(&profile.id);
    Ok(())
}

type App<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub async fn list_profiles(app: App<'_>) -> Result<Vec<ProfileDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || list(app.inner())).await
}

#[tauri::command]
pub async fn switch_profile(app: App<'_>, id: String) -> Result<Vec<ProfileDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        app.inner().switch_profile(&id)?;
        list(app.inner())
    })
    .await
}

#[tauri::command]
pub async fn profile_reuse_options(app: App<'_>, id: String) -> Result<ReuseOptions, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || reuse_options(app.inner(), &id)).await
}

#[tauri::command]
pub async fn create_profile(
    app: App<'_>,
    name: String,
    reuse: Vec<ReuseFrom>,
) -> Result<ProfileDto, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || create(app.inner(), &name, &reuse)).await
}

#[tauri::command]
pub async fn rename_profile(
    app: App<'_>,
    id: String,
    name: String,
) -> Result<Vec<ProfileDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        app.inner().profiles.rename(&id, &name)?;
        list(app.inner())
    })
    .await
}

#[tauri::command]
pub async fn delete_profile(app: App<'_>, id: String) -> Result<Vec<ProfileDto>, AppError> {
    let app = blocking::Handle::new(&app);
    blocking::run(move || {
        delete(app.inner(), &id)?;
        list(app.inner())
    })
    .await
}

/// The github.com accounts `gh` is signed in to (`gh auth status`).
#[tauri::command]
pub async fn gh_accounts() -> Result<Vec<String>, AppError> {
    blocking::run(|| Ok(pr_nexus_providers::github::gh_accounts())).await
}
