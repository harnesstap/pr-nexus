import { beforeEach, expect, test } from "vitest";
import type { SlackWorkspace } from "../api";
import { chosenWorkspace, csvHasWorkspaceColumn, lastWorkspace, rememberWorkspace, workspaceDomain, workspaceLabel } from "./workspaces";

const ws = (id: number, domain: string, partial: Partial<SlackWorkspace> = {}): SlackWorkspace => ({
  id,
  team_name: `${domain[0]?.toUpperCase()}${domain.slice(1)}`,
  domain,
  user_name: "ada",
  signed_in: true,
  needs_sign_in: false,
  ...partial,
});

beforeEach(() => window.localStorage.clear());

test("no workspace connected chooses nothing", () => {
  expect(chosenWorkspace([], "", null)).toBe("");
  expect(chosenWorkspace([], "acme", "acme")).toBe("");
});

test("a single workspace is preselected", () => {
  expect(chosenWorkspace([ws(1, "acme")], "", null)).toBe("acme");
  expect(chosenWorkspace([ws(1, "acme")], "", "widgets")).toBe("acme");
});

test("several workspaces prefer the pick, then the last used, then the first", () => {
  const list = [ws(1, "acme"), ws(2, "widgets")];
  expect(chosenWorkspace(list, "", null)).toBe("acme");
  expect(chosenWorkspace(list, "", "widgets")).toBe("widgets");
  expect(chosenWorkspace(list, "acme", "widgets")).toBe("acme");
  expect(chosenWorkspace(list, "", "gone")).toBe("acme");
});

test("the last used workspace is remembered per machine", () => {
  expect(lastWorkspace()).toBeNull();
  rememberWorkspace("");
  expect(lastWorkspace()).toBeNull();
  rememberWorkspace("widgets");
  expect(lastWorkspace()).toBe("widgets");
  expect(chosenWorkspace([ws(1, "acme"), ws(2, "widgets")], "")).toBe("widgets");
});

test("labels show name and host, and flag an expired sign-in", () => {
  expect(workspaceLabel(ws(1, "acme"))).toBe("Acme (acme.slack.com)");
  expect(workspaceLabel(ws(1, "acme", { needs_sign_in: true, signed_in: false }))).toBe("Acme (acme.slack.com) · needs sign-in");
  expect(workspaceLabel(ws(1, "acme", { team_name: "" }))).toBe("acme.slack.com");
});

test("hosts and URLs reduce to the stored domain", () => {
  expect(workspaceDomain("acme")).toBe("acme");
  expect(workspaceDomain(" Acme.slack.com ")).toBe("acme");
  expect(workspaceDomain("https://acme.slack.com/")).toBe("acme");
  expect(chosenWorkspace([ws(1, "acme"), ws(2, "widgets")], "widgets.slack.com", null)).toBe("widgets");
});

test("the CSV header decides whether rows carry their own workspace", () => {
  expect(csvHasWorkspaceColumn("github_org,github_team_slug\nacme,web")).toBe(false);
  expect(csvHasWorkspaceColumn("github_org,github_team_slug,slack_workspace\nacme,web,acme")).toBe(true);
  expect(csvHasWorkspaceColumn('\n"github_org","Slack_Workspace"\r\nacme,acme')).toBe(true);
  expect(csvHasWorkspaceColumn("")).toBe(false);
});
