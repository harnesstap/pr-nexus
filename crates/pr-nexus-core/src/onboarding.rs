//! Wizard progress stored in `onboarding.state`, and where a launch should resume.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::providers::SetupStep;
use crate::settings::{self, ONBOARDING};
use crate::store::Store;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnboardingState {
    pub step: String,
    pub completed: Vec<String>,
    #[serde(default)]
    pub mapping_skipped: bool,
    #[serde(default)]
    pub slack_skipped: bool,
    #[serde(default)]
    pub finished_at: Option<String>,
}

impl OnboardingState {
    pub fn fresh() -> Self {
        Self {
            step: SetupStep::ChooseVcs.as_str().to_owned(),
            completed: Vec::new(),
            mapping_skipped: false,
            slack_skipped: false,
            finished_at: None,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished_at
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    }
}

/// Cheap checks the shell fills in before calling [`resume_step`].
#[derive(Debug, Clone, Copy)]
pub struct StepValidity {
    pub vcs_selected: bool,
    pub chat_selected: bool,
    pub github_verified: bool,
    pub orgs_chosen: bool,
    pub slack_ok: bool,
    pub mapping_ok: bool,
}

impl StepValidity {
    pub fn holds(self, step: SetupStep) -> bool {
        match step {
            SetupStep::ChooseVcs => self.vcs_selected,
            SetupStep::ChooseChat => self.chat_selected,
            SetupStep::ConnectVcs => self.github_verified,
            SetupStep::ChooseVcsScope => self.orgs_chosen,
            SetupStep::ConnectChat => self.slack_ok,
            SetupStep::ChatMapping => self.mapping_ok,
            SetupStep::Done => true,
        }
    }
}

pub fn load(store: &Store) -> Result<OnboardingState, CoreError> {
    match store.get_setting(ONBOARDING.to_owned())? {
        None => Ok(OnboardingState::fresh()),
        Some(entry) => serde_json::from_str(&entry.value_json)
            .map_err(|err| CoreError::Store(format!("decode onboarding: {err}"))),
    }
}

pub fn save(store: &Store, state: &OnboardingState) -> Result<(), CoreError> {
    settings::put_json(store, ONBOARDING, state)
}

/// Resume at the stored step. A completed step that no longer validates wins,
/// so the wizard returns to the first broken step instead of skipping it.
pub fn resume_step(
    steps: &[SetupStep],
    saved: Option<&OnboardingState>,
    validity: StepValidity,
) -> SetupStep {
    if let Some(saved) = saved {
        for step in steps {
            if *step == SetupStep::Done {
                break;
            }
            let completed = saved.completed.iter().any(|item| item == step.as_str());
            if completed && !validity.holds(*step) {
                return *step;
            }
        }
        if let Some(step) = SetupStep::parse(&saved.step)
            && steps.contains(&step)
        {
            return step;
        }
    }
    steps.first().copied().unwrap_or(SetupStep::ChooseVcs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ChatKind, VcsKind};
    use crate::providers::ProviderRegistry;

    fn steps() -> Vec<SetupStep> {
        ProviderRegistry::v1().setup_steps(VcsKind::GitHub, ChatKind::Slack)
    }

    fn valid() -> StepValidity {
        StepValidity {
            vcs_selected: true,
            chat_selected: true,
            github_verified: true,
            orgs_chosen: true,
            slack_ok: true,
            mapping_ok: true,
        }
    }

    fn state(step: SetupStep, completed: &[&str]) -> OnboardingState {
        OnboardingState {
            step: step.as_str().to_owned(),
            completed: completed.iter().map(|item| (*item).to_owned()).collect(),
            mapping_skipped: false,
            slack_skipped: false,
            finished_at: None,
        }
    }

    #[test]
    fn fresh_state_starts_at_the_first_step() {
        assert_eq!(resume_step(&steps(), None, valid()), SetupStep::ChooseVcs);
    }

    #[test]
    fn resume_uses_the_stored_step_when_completed_steps_still_validate() {
        let saved = state(
            SetupStep::ChooseVcsScope,
            &["choose_vcs", "choose_chat", "connect_vcs"],
        );
        assert_eq!(
            resume_step(&steps(), Some(&saved), valid()),
            SetupStep::ChooseVcsScope
        );
    }

    #[test]
    fn resume_falls_back_to_the_first_invalid_completed_step() {
        let saved = state(
            SetupStep::ConnectChat,
            &[
                "choose_vcs",
                "choose_chat",
                "connect_vcs",
                "choose_vcs_scope",
            ],
        );
        let mut validity = valid();
        validity.github_verified = false;
        assert_eq!(
            resume_step(&steps(), Some(&saved), validity),
            SetupStep::ConnectVcs
        );
    }

    #[test]
    fn state_round_trips_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("onboarding.db")).unwrap();
        let mut saved = state(SetupStep::ChatMapping, &["choose_vcs"]);
        saved.mapping_skipped = true;
        saved.slack_skipped = true;
        save(&store, &saved).unwrap();
        assert_eq!(load(&store).unwrap(), saved);
    }
}
