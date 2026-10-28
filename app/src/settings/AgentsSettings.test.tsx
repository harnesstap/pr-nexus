import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { AgentsSettings } from "./AgentsSettings";
import { FAKE_MCP_TOKEN, fakeMcp } from "./fakes";
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

test("local writes and Slack sends are disabled and explained", async () => {
  render(<AgentsSettings api={fakeMcp({ enabled: true })} />);
  for (const name of ["Local writes", "Slack sends"]) {
    const toggle = await screen.findByRole("switch", { name });
    expect(toggle).toBeDisabled();
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(toggle).toHaveAccessibleDescription(/Coming soon/);
  }
  expect(screen.getByRole("switch", { name: "Reads" })).toBeEnabled();
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
