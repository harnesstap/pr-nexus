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
}

/** Slack sign-in is not available yet, so that screen can be skipped; mapping is optional. */
export function showSkip(step: StepId): boolean {
  return step === "connect_chat" || step === "chat_mapping";
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
      return true;
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
