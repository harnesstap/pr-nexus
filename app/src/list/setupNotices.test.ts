import { beforeEach, expect, test } from "vitest";
import type { McpClientState, SlackWorkspace } from "../api";
import { dismissSetupNotice, reconcileDismissed, setupProblems } from "./setupNotices";

const cursor: McpClientState = {
  client: "cursor",
  status: "outdated",
  path: "/Users/ada/.cursor/mcp.json",
  display_path: "~/.cursor/mcp.json",
  exists: true,
};
const claude: McpClientState = {
  client: "claude-code",
  status: "added",
  path: "/Users/ada/.claude.json",
  display_path: "~/.claude.json",
  exists: true,
};

const acme: SlackWorkspace = {
  id: 1,
  team_name: "Acme",
  domain: "acme",
  user_name: "ada",
  signed_in: false,
  needs_sign_in: false,
  team_id: "T0ACME0001",
  enabled: true,
  priority: 0,
  check: "missing",
};

beforeEach(() => window.localStorage.clear());

test("outdated MCP configs and a missing Slack grant are one notice each", () => {
  const problems = setupProblems({
    clients: [claude, cursor],
    workspaces: [acme, { ...acme, id: 2, team_id: "T0ACME0002", team_name: "Acme Labs", check: "connected", enabled: true }],
  });
  expect(problems.map((problem) => problem.id)).toEqual(["mcp-outdated:cursor", "slack-missing:T0ACME0001"]);
  expect(problems[0]?.message).toBe("Cursor has an outdated PR Nexus config.");
  expect(problems[0]?.target).toBe("client");
  expect(problems[0]?.client).toBe("cursor");
  expect(problems[1]?.message).toBe("Acme Slack is missing a grant.");
  expect(problems[1]?.target).toBe("slack");
});

test("a disabled workspace and a current config stay quiet", () => {
  expect(
    setupProblems({
      clients: [claude],
      workspaces: [{ ...acme, enabled: false }],
    }),
  ).toEqual([]);
});

test("dismissing hides a problem until it is fixed, then it can show again", () => {
  dismissSetupNotice("mcp-outdated:cursor");
  expect(reconcileDismissed(["mcp-outdated:cursor"]).has("mcp-outdated:cursor")).toBe(true);
  expect(reconcileDismissed([]).has("mcp-outdated:cursor")).toBe(false);
  expect(reconcileDismissed(["mcp-outdated:cursor"]).has("mcp-outdated:cursor")).toBe(false);
});
