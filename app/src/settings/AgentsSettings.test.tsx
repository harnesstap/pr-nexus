import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { AgentsSettings } from "./AgentsSettings";
import { FAKE_MCP_TOKEN, fakeMcp, sampleActivity } from "./fakes";
import { mcpConfigSnippet, parsePort } from "./tabs";

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
  expect(save).toHaveBeenCalledWith({ enabled: true, reads: true, local_writes: false, slack_sends: false, port: null });
  expect(screen.getByText("http://127.0.0.1:43117/mcp")).toBeInTheDocument();
  expect(screen.getByLabelText("Token, masked")).toHaveTextContent("pnx_••••••••••••7f3e");
  expect(document.body.textContent).not.toContain(FAKE_MCP_TOKEN);
});

test("local writes can be turned on; Slack sends stays disabled and explained", async () => {
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
  expect(slack).toHaveAccessibleDescription(/Coming soon/);
  expect(screen.getByRole("switch", { name: "Reads" })).toBeEnabled();
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
  await screen.findByText("Copied client config.");
  expect(writeText).toHaveBeenLastCalledWith(mcpConfigSnippet("http://127.0.0.1:43117/mcp", FAKE_MCP_TOKEN));
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
    settings: { enabled: true, reads: true, local_writes: false, slack_sends: false, port: 40100 },
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
