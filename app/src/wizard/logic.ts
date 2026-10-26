import type { StepId } from "../api";

export interface StepModel {
  vcs: "github" | null;
  chat: "slack" | null;
  githubConnected: boolean;
  orgCount: number;
  personal: boolean;
  syncSettled: boolean;
}

/** Slack sign-in is not built until M10, so that screen can be skipped. */
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
