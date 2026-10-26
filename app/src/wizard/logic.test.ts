import { expect, test } from "vitest";
import { backEnabled, nextEnabled, nextHint, nextLabel, showSkip, type StepModel } from "./logic";

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
  expect(nextEnabled("connect_chat", ready)).toBe(true);
  expect(nextEnabled("done", { ...ready, syncSettled: false })).toBe(false);
  expect(nextEnabled("done", ready)).toBe(true);
});

test("a disabled Next always explains itself", () => {
  expect(nextHint("connect_vcs", { ...ready, githubConnected: false })).toMatch(/Connect GitHub/);
  expect(nextHint("choose_vcs_scope", { ...ready, orgCount: 0, personal: false })).toMatch(/at least one org/);
  expect(nextHint("connect_vcs", ready)).toBeNull();
  expect(nextEnabled("chat_mapping", { ...ready, csvFilled: false })).toBe(true);
});

test("mapping Next imports only when CSV was pasted", () => {
  expect(nextLabel("chat_mapping", { ...ready, csvFilled: true })).toBe("Import & next");
  expect(nextLabel("chat_mapping", { ...ready, csvFilled: false })).toBe("Next");
  expect(nextLabel("connect_vcs", ready)).toBe("Next");
});
