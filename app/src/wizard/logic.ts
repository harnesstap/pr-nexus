import type { StepId } from "../api";

export interface StepModel {
  vcs: "github" | null;
  chat: "slack" | null;
  githubConnected: boolean;
  orgCount: number;
  personal: boolean;
  syncSettled: boolean;
  /** True when the mapping step has CSV text to import. */
  csvFilled?: boolean;
  /** At least one Slack workspace is signed in. */
  slackConnected?: boolean;
}

/**
 * Mapping is optional. Slack can be skipped too, so PR Nexus stays usable
 * when a workspace has no approved Slack app yet (§7.13 R7).
 */
export function showSkip(step: StepId): boolean {
  return step === "connect_chat" || step === "chat_mapping";
}

/**
 * Steps with a single possible answer (GitHub, Slack) are not shown: the wizard
 * records the choice itself, so setup is five screens instead of seven.
 */
export const AUTO_STEPS: readonly StepId[] = ["choose_vcs", "choose_chat"];

export function visibleSteps(steps: readonly StepId[]): StepId[] {
  return steps.filter((step) => !AUTO_STEPS.includes(step));
}

/** Where to open: the resume step, or the first shown step when it points at a hidden one. */
export function startIndex(steps: readonly StepId[], resume: StepId): number {
  return Math.max(0, steps.indexOf(resume));
}

export type GithubMode = "gh_cli" | "pat" | "device";

/** Why Connect GitHub is disabled, or null when it can run. */
export function connectBlock(mode: GithubMode, input: { pat: string; ghReady: boolean; oauthConfigured: boolean }): string | null {
  switch (mode) {
    case "gh_cli":
      return input.ghReady ? null : "The gh CLI has no token yet. Run gh auth login, or paste a token instead.";
    case "pat":
      return input.pat.trim() ? null : "Paste a token above to connect.";
    case "device":
      return input.oauthConfigured
        ? null
        : "Device sign-in needs a GitHub OAuth app client id, set in github.oauth_client_id or the PR_NEXUS_GITHUB_OAUTH_CLIENT_ID environment variable. None ships with PR Nexus.";
    default: {
      const unreachable: never = mode;
      return unreachable;
    }
  }
}

export function backEnabled(index: number): boolean {
  return index > 0;
}

export function nextEnabled(step: StepId, model: StepModel): boolean {
  switch (step) {
    case "choose_vcs":
      return model.vcs === "github";
    case "choose_chat":
      return model.chat === "slack";
    case "connect_vcs":
      return model.githubConnected;
    case "choose_vcs_scope":
      return model.orgCount > 0 || model.personal;
    case "connect_chat":
      return model.slackConnected === true;
    case "chat_mapping":
      return true;
    case "done":
      return model.syncSettled;
    default: {
      const unreachable: never = step;
      return unreachable;
    }
  }
}

/** Why Next is disabled, shown under the footer. Null when Next is enabled. */
export function nextHint(step: StepId, model: StepModel): string | null {
  if (nextEnabled(step, model)) {
    return null;
  }
  switch (step) {
    case "connect_vcs":
      return "Connect GitHub above to continue.";
    case "choose_vcs_scope":
      return "Pick at least one org, or include your personal repos, to continue.";
    case "connect_chat":
      return "Sign in to a Slack workspace above, or Skip and connect later in Settings.";
    case "done":
      return "Waiting for the first sync to finish…";
    default:
      return null;
  }
}

/** The mapping step imports pasted CSV on Next; with nothing pasted Next just continues. */
export function nextLabel(step: StepId, model: StepModel): string {
  if (step === "chat_mapping" && model.csvFilled) {
    return "Import & next";
  }
  return "Next";
}

export function stepTitle(step: StepId): string {
  switch (step) {
    case "choose_vcs":
      return "Git provider";
    case "choose_chat":
      return "Communication provider";
    case "connect_vcs":
      return "Connect GitHub";
    case "choose_vcs_scope":
      return "Orgs to track";
    case "connect_chat":
      return "Connect Slack";
    case "chat_mapping":
      return "Team → channel mapping";
    case "done":
      return "Done";
    default: {
      const unreachable: never = step;
      return unreachable;
    }
  }
}
