import { beforeEach, expect, test } from "vitest";
import type { SlackWorkspace } from "../api";
import {
  chosenWorkspace,
  csvHasWorkspaceColumn,
  findWorkspace,
  isTeamId,
  lastWorkspace,
  rememberWorkspace,
  targetWorkspaceName,
  workspaceDomain,
  workspaceLabel,
} from "./workspaces";

const ws = (id: number, domain: string, partial: Partial<SlackWorkspace> = {}): SlackWorkspace => ({
  id,
  team_name: `${domain[0]?.toUpperCase()}${domain.slice(1)}`,
  domain,
  user_name: "ada",
  signed_in: true,
  needs_sign_in: false,
  team_id: `T0ACME000${id}`,
  enabled: true,
  priority: id - 1,
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
  expect(workspaceLabel(ws(1, "acme", { needs_sign_in: true, team_id: "T0ACME0002", enabled: true, priority: 1, signed_in: false }))).toBe("Acme (acme.slack.com) · needs sign-in");
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

test("a team ID routes exactly; a domain is only a hint", () => {
  const acme = ws(1, "acme");
  const labs = ws(2, "acme-labs");
  expect(isTeamId("T0ACME0002")).toBe(true);
  expect(isTeamId("acme-labs")).toBe(false);
  expect(workspaceDomain("t0acme0002")).toBe("T0ACME0002");
  expect(findWorkspace([acme, labs], "T0ACME0002")).toBe(labs);
  expect(targetWorkspaceName({ workspace: "acme", workspace_name: "Acme" })).toBe("Acme");
  expect(targetWorkspaceName({ workspace: "acme" })).toBe("acme");
});
