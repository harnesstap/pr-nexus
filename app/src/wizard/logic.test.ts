import { expect, test } from "vitest";
import { backEnabled, connectBlock, nextEnabled, nextHint, nextLabel, showSkip, startIndex, visibleSteps, type StepModel } from "./logic";

const ready: StepModel = {
  vcs: "github",
  chat: "slack",
  githubConnected: true,
  orgCount: 1,
  personal: false,
  syncSettled: true,
};

test("back is disabled only on the first step", () => {
  expect(backEnabled(0)).toBe(false);
  expect(backEnabled(1)).toBe(true);
});

test("skip is offered on Slack connect and on mapping", () => {
  expect(showSkip("choose_vcs")).toBe(false);
  expect(showSkip("choose_chat")).toBe(false);
  expect(showSkip("connect_vcs")).toBe(false);
  expect(showSkip("choose_vcs_scope")).toBe(false);
  expect(showSkip("connect_chat")).toBe(true);
  expect(showSkip("chat_mapping")).toBe(true);
  expect(showSkip("done")).toBe(false);
});

test("next stays disabled until the step condition holds", () => {
  expect(nextEnabled("choose_vcs", { ...ready, vcs: null })).toBe(false);
  expect(nextEnabled("choose_chat", { ...ready, chat: null })).toBe(false);
  expect(nextEnabled("connect_vcs", { ...ready, githubConnected: false })).toBe(false);
  expect(nextEnabled("choose_vcs_scope", { ...ready, orgCount: 0, personal: false })).toBe(false);
  expect(nextEnabled("choose_vcs_scope", { ...ready, orgCount: 0, personal: true })).toBe(true);
  expect(nextEnabled("connect_chat", ready)).toBe(false);
  expect(nextEnabled("connect_chat", { ...ready, slackConnected: true })).toBe(true);
  expect(nextEnabled("done", { ...ready, syncSettled: false })).toBe(false);
  expect(nextEnabled("done", ready)).toBe(true);
});

test("a disabled Next always explains itself", () => {
  expect(nextHint("connect_vcs", { ...ready, githubConnected: false })).toMatch(/Connect GitHub/);
  expect(nextHint("choose_vcs_scope", { ...ready, orgCount: 0, personal: false })).toMatch(/at least one org/);
  expect(nextHint("connect_vcs", ready)).toBeNull();
  expect(nextHint("connect_chat", ready)).toMatch(/Sign in to a Slack workspace/);
  expect(nextEnabled("chat_mapping", { ...ready, csvFilled: false })).toBe(true);
});

test("mapping Next imports only when CSV was pasted", () => {
  expect(nextLabel("chat_mapping", { ...ready, csvFilled: true })).toBe("Import & next");
  expect(nextLabel("chat_mapping", { ...ready, csvFilled: false })).toBe("Next");
  expect(nextLabel("connect_vcs", ready)).toBe("Next");
});

test("single-choice steps are hidden and a resume on one opens the first shown step", () => {
  const all = ["choose_vcs", "connect_vcs", "choose_vcs_scope", "choose_chat", "connect_chat", "chat_mapping", "done"] as const;
  const shown = visibleSteps(all);
  expect(shown).toEqual(["connect_vcs", "choose_vcs_scope", "connect_chat", "chat_mapping", "done"]);
  expect(startIndex(shown, "choose_chat")).toBe(0);
  expect(startIndex(shown, "connect_chat")).toBe(2);
});

test("Connect GitHub needs input for the chosen mode", () => {
  const base = { pat: "", ghReady: false, oauthConfigured: false };
  expect(connectBlock("pat", base)).toMatch(/Paste a token/);
  expect(connectBlock("pat", { ...base, pat: "  " })).not.toBeNull();
  expect(connectBlock("pat", { ...base, pat: "github_pat_x" })).toBeNull();
  expect(connectBlock("gh_cli", base)).toMatch(/gh auth login/);
  expect(connectBlock("gh_cli", { ...base, ghReady: true })).toBeNull();
  expect(connectBlock("device", base)).toMatch(/PR_NEXUS_GITHUB_OAUTH_CLIENT_ID/);
  expect(connectBlock("device", { ...base, oauthConfigured: true })).toBeNull();
});
