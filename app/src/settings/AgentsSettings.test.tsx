import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { AgentsSettings } from "./AgentsSettings";
import type { SlackProposal } from "../api";
import { FAKE_MCP_TOKEN, FAKE_SLACK_WORKSPACES, fakeMcp, sampleActivity, sampleSlackActivity } from "./fakes";
import { mcpClientSnippet } from "./mcpClients";
import { parsePort } from "./tabs";

const writeText = vi.fn(() => Promise.resolve());

beforeEach(() => {
  Object.assign(navigator, { clipboard: { writeText } });
});

afterEach(() => {
  writeText.mockClear();
});

test("is off by default and starts the server when enabled", async () => {
  const api = fakeMcp();
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const master = await screen.findByRole("switch", { name: "Enable MCP server" });
  expect(master).toHaveAttribute("aria-checked", "false");
  expect(screen.getByText("○ Off")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Copy token" })).toBeNull();

  fireEvent.click(master);
  expect(await screen.findByText("● Running on port 43117")).toBeInTheDocument();
  expect(save).toHaveBeenCalledWith({ enabled: true, reads: true, local_writes: false, slack_sends: false, slack_reads: false, slack_private_search: false, slack_workspaces: [], slack_auto_send_channels: [], slack_send_mode: "allowlist", slack_elicit: true, port: null });
  expect(screen.getByText("http://127.0.0.1:43117/mcp")).toBeInTheDocument();
  expect(screen.getByLabelText("Token, masked")).toHaveTextContent("pnx_••••••••••••7f3e");
  expect(document.body.textContent).not.toContain(FAKE_MCP_TOKEN);
});

test("local writes can be turned on; Slack sends waits for Slack reads", async () => {
  const api = fakeMcp({ enabled: true });
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const writes = await screen.findByRole("switch", { name: "Local writes" });
  expect(writes).toBeEnabled();
  expect(writes).toHaveAttribute("aria-checked", "false");

  fireEvent.click(writes);
  await screen.findByText(/Agents cannot delete projects/);
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ local_writes: true, slack_sends: false }));
  expect(screen.getByRole("switch", { name: "Local writes" })).toHaveAttribute("aria-checked", "true");

  const slack = screen.getByRole("switch", { name: "Slack sends" });
  expect(slack).toBeDisabled();
  expect(slack).toHaveAttribute("aria-checked", "false");
  expect(slack).toHaveAccessibleDescription(/Turn on Slack reads first/);
  expect(screen.getByRole("switch", { name: "Reads" })).toBeEnabled();
});

test("Slack reads is off by default and exposes only the workspaces ticked", async () => {
  const api = fakeMcp({ enabled: true }, [], FAKE_SLACK_WORKSPACES);
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const reads = await screen.findByRole("switch", { name: "Slack reads" });
  expect(reads).toHaveAttribute("aria-checked", "false");
  expect(screen.queryByRole("group", { name: "Workspaces agents may read" })).toBeNull();
  expect(screen.getByRole("switch", { name: "Slack private search" })).toBeDisabled();

  fireEvent.click(reads);
  const picker = await screen.findByRole("group", { name: "Workspaces agents may read" });
  const boxes = within(picker).getAllByRole("checkbox");
  expect(boxes).toHaveLength(2);
  boxes.forEach((box) => expect(box).not.toBeChecked());

  fireEvent.click(within(picker).getByRole("checkbox", { name: /Acme Labs/ }));
  await vi.waitFor(() => expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_reads: true, slack_workspaces: ["T0ACMELAB1"] })));
  await vi.waitFor(() => expect(within(picker).getByRole("checkbox", { name: /Acme Labs/ })).toBeChecked());
  expect(within(picker).getByRole("checkbox", { name: /^Acmeacme\.slack/ })).not.toBeChecked();
});

test("activity lists calls newest first with time, client, tool, summary and outcome", async () => {
  const now = new Date();
  render(<AgentsSettings api={fakeMcp({ enabled: true, local_writes: true }, sampleActivity(now))} />);
  const list = await screen.findByRole("list", { name: "Agent activity" });
  const items = within(list).getAllByRole("listitem");
  expect(items).toHaveLength(7);
  expect(items[0]).toHaveTextContent("set_pr_note");
  expect(items[0]).toHaveTextContent("set note on acme/api#482");
  expect(items[0]).toHaveTextContent("claude-code");
  expect(items[0]).toHaveTextContent("allowed");
  expect(items[4]).toHaveTextContent("denied");
  expect(items[6]).toHaveTextContent("listed projects");
});

test("activity filters by outcome", async () => {
  const api = fakeMcp({ enabled: true }, sampleActivity());
  const read = vi.spyOn(api, "mcpActivity");
  render(<AgentsSettings api={api} />);
  await screen.findByRole("list", { name: "Agent activity" });
  fireEvent.change(screen.getByRole("combobox", { name: "Filter by outcome" }), { target: { value: "denied" } });
  await vi.waitFor(() => expect(within(screen.getByRole("list", { name: "Agent activity" })).getAllByRole("listitem")).toHaveLength(1));
  expect(read).toHaveBeenLastCalledWith("denied");
  expect(screen.getByRole("list", { name: "Agent activity" })).toHaveTextContent("set_pr_starred");

  fireEvent.change(screen.getByRole("combobox", { name: "Filter by outcome" }), { target: { value: "all" } });
  await vi.waitFor(() => expect(within(screen.getByRole("list", { name: "Agent activity" })).getAllByRole("listitem")).toHaveLength(7));
  expect(read).toHaveBeenLastCalledWith(undefined);
});

test("clearing the log asks first", async () => {
  const api = fakeMcp({ enabled: true }, sampleActivity());
  const clear = vi.spyOn(api, "clearMcpActivity");
  render(<AgentsSettings api={api} />);
  await screen.findByRole("list", { name: "Agent activity" });
  fireEvent.click(screen.getByRole("button", { name: "Clear log…" }));
  fireEvent.click(within(screen.getByRole("group", { name: "Confirm clear log" })).getByRole("button", { name: "Cancel" }));
  expect(clear).not.toHaveBeenCalled();
  expect(screen.getByRole("list", { name: "Agent activity" })).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Clear log…" }));
  fireEvent.click(within(screen.getByRole("group", { name: "Confirm clear log" })).getByRole("button", { name: "Clear log" }));
  expect(await screen.findByText("No agent has called a tool yet.")).toBeInTheDocument();
  expect(clear).toHaveBeenCalledOnce();
  expect(screen.queryByRole("list", { name: "Agent activity" })).toBeNull();
});

test("copies the real token and a config snippet with it", async () => {
  render(<AgentsSettings api={fakeMcp({ enabled: true })} />);
  fireEvent.click(await screen.findByRole("button", { name: "Copy token" }));
  await screen.findByText("Copied token.");
  expect(writeText).toHaveBeenLastCalledWith(FAKE_MCP_TOKEN);

  fireEvent.click(screen.getByRole("button", { name: "Copy client config" }));
  await screen.findByText("Copied Claude Desktop config.");
  expect(writeText).toHaveBeenLastCalledWith(mcpClientSnippet("claude-desktop", "http://127.0.0.1:43117/mcp", FAKE_MCP_TOKEN).text);
  expect(screen.getByRole("tabpanel")).not.toHaveTextContent(FAKE_MCP_TOKEN);

  fireEvent.click(screen.getByRole("tab", { name: "Codex" }));
  expect(window.localStorage.getItem("pr-nexus.mcp-client-tab")).toBe("codex");
  expect(screen.getByRole("tabpanel")).toHaveTextContent("~/.codex/config.toml");
  fireEvent.click(screen.getByRole("button", { name: "Copy client config" }));
  await screen.findByText("Copied Codex config.");
  expect(writeText).toHaveBeenLastCalledWith(mcpClientSnippet("codex", "http://127.0.0.1:43117/mcp", FAKE_MCP_TOKEN).text);
  window.localStorage.clear();
});

test("regenerating the token asks first", async () => {
  const api = fakeMcp({ enabled: true });
  const regenerate = vi.spyOn(api, "regenerateMcpToken");
  render(<AgentsSettings api={api} />);
  fireEvent.click(await screen.findByRole("button", { name: "Regenerate token…" }));
  const confirm = screen.getByRole("group", { name: "Confirm new token" });
  fireEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
  expect(regenerate).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Regenerate token…" }));
  await act(async () => {
    fireEvent.click(within(screen.getByRole("group", { name: "Confirm new token" })).getByRole("button", { name: "Regenerate token" }));
  });
  expect(regenerate).toHaveBeenCalledOnce();
  expect(screen.getByLabelText("Token, masked")).toHaveTextContent("c0de");
});

test("a bad port is refused before saving; a server error is shown", async () => {
  const api = fakeMcp({ enabled: true });
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const input = await screen.findByRole("textbox", { name: "Port" });
  fireEvent.change(input, { target: { value: "80" } });
  fireEvent.click(screen.getByRole("button", { name: "Apply port" }));
  expect(screen.getByRole("alert")).toHaveTextContent("1024 to 65535");
  expect(save).not.toHaveBeenCalled();

  save.mockResolvedValueOnce({
    settings: { enabled: true, reads: true, local_writes: false, slack_sends: false, slack_reads: false, slack_private_search: false, slack_workspaces: [], slack_auto_send_channels: [], slack_send_mode: "allowlist", slack_elicit: true, port: 40100 },
    status: { state: "error", url: null, port: 40100, error: "Port 40100 is already in use on 127.0.0.1." },
    token_masked: "pnx_••••••••••••7f3e",
  });
  fireEvent.change(input, { target: { value: "40100" } });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(await screen.findByText(/Not running: Port 40100 is already in use/)).toBeInTheDocument();
  expect(save).toHaveBeenCalledWith(expect.objectContaining({ port: 40100 }));
});

test("ports parse to null, a number or an error", () => {
  expect(parsePort(" ")).toEqual({ port: null });
  expect(parsePort("40100")).toEqual({ port: 40100 });
  expect(parsePort("1023")).toHaveProperty("error");
  expect(parsePort("70000")).toHaveProperty("error");
  expect(parsePort("12a")).toHaveProperty("error");
});

test("Slack private search needs Slack reads and saves on its own", async () => {
  const api = fakeMcp({ enabled: true }, [], FAKE_SLACK_WORKSPACES);
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const priv = await screen.findByRole("switch", { name: "Slack private search" });
  expect(priv).toBeDisabled();
  expect(priv).toHaveAccessibleDescription(/Turn on Slack reads first/);

  fireEvent.click(screen.getByRole("switch", { name: "Slack reads" }));
  await vi.waitFor(() => expect(screen.getByRole("switch", { name: "Slack private search" })).toBeEnabled());
  fireEvent.click(screen.getByRole("switch", { name: "Slack private search" }));
  await vi.waitFor(() =>
    expect(screen.getByRole("switch", { name: "Slack private search" })).toHaveAttribute("aria-checked", "true"),
  );
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_reads: true, slack_private_search: true, slack_sends: false }));
});

test("a workspace missing new scopes offers Sign in again, which clears the note", async () => {
  const stale = [FAKE_SLACK_WORKSPACES[0], { ...FAKE_SLACK_WORKSPACES[1], scopes_missing: ["reactions:read", "canvases:read"] }];
  const api = fakeMcp({ enabled: true, slack_reads: true }, [], stale);
  const connect = vi.spyOn(api, "slackConnect");
  render(<AgentsSettings api={api} />);
  const again = await screen.findByRole("button", { name: "Sign in again to Acme Labs" });
  expect(again).toHaveAccessibleDescription(/needs new Slack permissions/);
  expect(screen.queryByRole("button", { name: "Sign in again to Acme" })).toBeNull();

  fireEvent.click(again);
  await vi.waitFor(() => expect(screen.queryByRole("button", { name: "Sign in again to Acme Labs" })).toBeNull());
  expect(connect).toHaveBeenCalledTimes(1);
});

function proposal(id: number, patch: Partial<SlackProposal> = {}): SlackProposal {
  return {
    id,
    created_at: new Date().toISOString(),
    client: "claude-code",
    tool: "slack_send_message",
    workspace: { team_id: "T0ACME0001", qualifier: "T0ACME0001" },
    channel_id: "T0ACME0001/C0ACME00001",
    arguments: { channel_id: "C0ACME00001", message: `Review please #${id}` },
    status: "pending",
    held: { reason: "not_listed" },
    auto_sent: false,
    result: null,
    error: null,
    decided_at: null,
    ...patch,
  };
}

test("Slack sends turns on and edits the auto-send channels", async () => {
  const api = fakeMcp({ enabled: true, slack_reads: true }, [], FAKE_SLACK_WORKSPACES);
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const sends = await screen.findByRole("switch", { name: "Slack sends" });
  expect(sends).toBeEnabled();
  expect(screen.queryByRole("textbox", { name: "Add auto-send channel" })).toBeNull();
  fireEvent.click(sends);
  const input = await screen.findByRole("textbox", { name: "Add auto-send channel" });
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_sends: true, slack_auto_send_channels: [] }));

  fireEvent.change(input, { target: { value: "C0ACME00001" } });
  fireEvent.click(screen.getByRole("button", { name: "Add" }));
  expect(screen.getByRole("alert")).toHaveTextContent(/qualified channel ID/);

  fireEvent.change(input, { target: { value: " T0ACME0001/C0ACME00001 " } });
  fireEvent.click(screen.getByRole("button", { name: "Add" }));
  await screen.findByRole("button", { name: "Remove T0ACME0001/C0ACME00001" });
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_auto_send_channels: ["T0ACME0001/C0ACME00001"] }));

  fireEvent.click(screen.getByRole("button", { name: "Remove T0ACME0001/C0ACME00001" }));
  await vi.waitFor(() => expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_auto_send_channels: [] })));
});

test("pending proposals are approved or rejected, with why each one waited", async () => {
  const held = proposal(2, { held: { reason: "read_other_workspace", workspaces: ["acme-labs"] } });
  const failed = proposal(3, { status: "failed", held: null, auto_sent: true, error: "not_in_channel" });
  const api = fakeMcp({ enabled: true, slack_reads: true, slack_sends: true }, [], FAKE_SLACK_WORKSPACES, [proposal(1), held, failed]);
  const decide = vi.spyOn(api, "decideMcpSlackProposal");
  render(<AgentsSettings api={api} />);
  const list = await screen.findByRole("list", { name: "Slack proposals" });
  expect(within(list).getByText("Channel not on the auto-send list")).toBeInTheDocument();
  expect(within(list).getByText(/this agent read acme-labs first/)).toBeInTheDocument();
  expect(within(list).getByText("Slack: not_in_channel")).toBeInTheDocument();
  expect(screen.getByText("(2 waiting)")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Approve proposal 1" }));
  await vi.waitFor(() => expect(screen.queryByRole("button", { name: "Approve proposal 1" })).toBeNull());
  expect(decide).toHaveBeenLastCalledWith(1, true);
  fireEvent.click(screen.getByRole("button", { name: "Reject proposal 2" }));
  await vi.waitFor(() => expect(screen.queryByRole("button", { name: "Reject proposal 2" })).toBeNull());
  expect(decide).toHaveBeenLastCalledWith(2, false);
  expect(within(list).getAllByText("sent")).toHaveLength(1);
  expect(within(list).getByText("rejected")).toBeInTheDocument();
});

test("approving needs Slack sends on; rejecting does not", async () => {
  const api = fakeMcp({ enabled: true, slack_reads: true, slack_sends: false }, [], FAKE_SLACK_WORKSPACES, [proposal(1)]);
  render(<AgentsSettings api={api} />);
  expect(await screen.findByRole("button", { name: "Approve proposal 1" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Reject proposal 1" })).toBeEnabled();
  expect(screen.getByText("Turn on Slack sends to approve.")).toBeInTheDocument();
});

test("Slack send modes show the allowlist only in allowlist mode and warn in automatic mode", async () => {
  const api = fakeMcp({ enabled: true, slack_reads: true, slack_sends: true, slack_auto_send_channels: ["T0ACME0001/C0123456789"] }, [], FAKE_SLACK_WORKSPACES);
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const allowlist = await screen.findByRole("radio", { name: /Channel allowlist/ });
  expect(allowlist).toBeChecked();
  expect(screen.getByRole("textbox", { name: "Add auto-send channel" })).toBeInTheDocument();
  expect(screen.queryByRole("note")).toBeNull();

  fireEvent.click(screen.getByRole("radio", { name: /Fully automatic/ }));
  expect(await screen.findByRole("note")).toHaveTextContent("sent immediately as you");
  expect(screen.queryByRole("textbox", { name: "Add auto-send channel" })).toBeNull();
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_send_mode: "automatic", slack_auto_send_channels: ["T0ACME0001/C0123456789"] }));

  fireEvent.click(screen.getByRole("radio", { name: /Approve everything/ }));
  await vi.waitFor(() => expect(screen.getByRole("radio", { name: /Approve everything/ })).toBeChecked());
  expect(screen.queryByRole("note")).toBeNull();
  expect(screen.queryByRole("textbox", { name: "Add auto-send channel" })).toBeNull();
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_send_mode: "approve_all" }));

  fireEvent.click(allowlist);
  // The saved allowlist survives the round trip through other modes.
  await screen.findByRole("button", { name: "Remove T0ACME0001/C0123456789" });
});

test("adds pr-nexus to a client and shows its status", async () => {
  const api = fakeMcp({ enabled: true }, [], [], [], { cursor: "outdated", opencode: "unreadable" });
  const add = vi.spyOn(api, "mcpAddToClient");
  const openFile = vi.spyOn(api, "mcpOpenClientConfig");
  window.localStorage.clear();
  render(<AgentsSettings api={api} />);

  expect(await screen.findByRole("status", { name: "Claude Desktop status" })).toHaveTextContent("Not added");
  expect(screen.getByRole("button", { name: "Open folder" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Add to Claude Desktop" }));
  expect(await screen.findByRole("button", { name: "Added ✓" })).toBeDisabled();
  expect(screen.getByRole("status", { name: "Claude Desktop status" })).toHaveTextContent("Added");
  expect(screen.getByText("Restart Claude Desktop to load it.")).toBeInTheDocument();
  expect(add).toHaveBeenLastCalledWith("claude-desktop");
  fireEvent.click(screen.getByRole("button", { name: "Open config file" }));
  expect(openFile).toHaveBeenLastCalledWith("claude-desktop");

  fireEvent.click(screen.getByRole("tab", { name: "Cursor" }));
  expect(screen.getByRole("status", { name: "Cursor status" })).toHaveTextContent("Added, but outdated");
  fireEvent.click(screen.getByRole("button", { name: "Update Cursor" }));
  expect(await screen.findByRole("button", { name: "Added ✓" })).toBeDisabled();

  fireEvent.click(screen.getByRole("tab", { name: "OpenCode" }));
  expect(screen.getByRole("button", { name: "Add to OpenCode" })).toBeDisabled();
  window.localStorage.clear();
});

test("Ask in the agent is on by default, saves when toggled, and hides in automatic mode", async () => {
  const api = fakeMcp({ enabled: true, slack_reads: true, slack_sends: true }, [], FAKE_SLACK_WORKSPACES);
  const save = vi.spyOn(api, "saveMcpSettings");
  render(<AgentsSettings api={api} />);
  const ask = await screen.findByRole("checkbox", { name: /Ask in the agent when it supports it/ });
  expect(ask).toBeChecked();
  fireEvent.click(ask);
  await vi.waitFor(() => expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ slack_elicit: false })));
  expect(await screen.findByRole("checkbox", { name: /Ask in the agent/ })).not.toBeChecked();
  fireEvent.click(screen.getByRole("radio", { name: /Fully automatic/ }));
  await vi.waitFor(() => expect(screen.queryByRole("checkbox", { name: /Ask in the agent/ })).toBeNull());
});

test("a proposal confirmed in the agent says where", async () => {
  const api = fakeMcp({ enabled: true, slack_reads: true, slack_sends: true }, [], FAKE_SLACK_WORKSPACES, [proposal(7, { status: "sent", decided_in: "claude-code", decided_at: new Date().toISOString() }), proposal(8, { status: "rejected", decided_in: "claude-code", decided_at: new Date().toISOString() })]);
  render(<AgentsSettings api={api} />);
  expect(await screen.findByText("Approved in claude-code")).toBeInTheDocument();
  expect(screen.getByText("Declined in claude-code")).toBeInTheDocument();
});

test("activity links Slack writes to the message and opens it in the browser", async () => {
  const api = fakeMcp({ enabled: true, slack_sends: true }, sampleSlackActivity());
  const open = vi.spyOn(api, "openExternal");
  render(<AgentsSettings api={api} />);
  const list = await screen.findByRole("list", { name: "Agent activity" });
  const [reply, sent, read] = within(list).getAllByRole("listitem");
  expect(reply).toHaveTextContent("replied in a thread in #release");
  fireEvent.click(within(reply).getByRole("button", { name: "↗ Open in Slack" }));
  expect(open).toHaveBeenCalledWith("https://acme.slack.com/archives/C0123456789/p1700000500000200?thread_ts=1700000000.000100&cid=C0123456789");
  fireEvent.click(within(sent).getByRole("button", { name: "↗ Open in Slack" }));
  expect(open).toHaveBeenLastCalledWith("https://acme.slack.com/archives/C0123456789/p1700000000000100");
  expect(within(read).queryByRole("button", { name: "↗ Open in Slack" })).toBeNull();
});
