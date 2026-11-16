import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import type { Api, SlackWorkspace } from "../api";
import { TOOLTIP_DELAY_MS } from "../Tooltip";
import { SlackPanel } from "./SlackPanel";

afterEach(() => vi.useRealTimers());

const acme: SlackWorkspace = { id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 };
const labs: SlackWorkspace = { id: 2, team_name: "Acme Labs", domain: "acme-labs", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0002", enabled: true, priority: 1 };

/** A backend that keeps the list like the Tauri commands do. */
function setup(start: SlackWorkspace[] = [labs, acme], fail = false) {
  let stored = start.map((item) => ({ ...item }));
  const calls = { enabled: [] as [string, boolean][], order: [] as string[][] };
  const sorted = () => [...stored].sort((a, b) => a.priority - b.priority);
  const api = {
    slackSetWorkspaceEnabled: (teamId: string, enabled: boolean) => {
      calls.enabled.push([teamId, enabled]);
      stored = stored.map((item) => (item.team_id === teamId ? { ...item, enabled } : item));
      return Promise.resolve(sorted());
    },
    slackSetWorkspaceOrder: (teamIds: string[]) => {
      calls.order.push(teamIds);
      if (fail) {
        return Promise.reject({ kind: "slack", message: "unknown workspace" });
      }
      stored = stored.map((item) => ({ ...item, priority: teamIds.indexOf(item.team_id) }));
      return Promise.resolve(sorted());
    },
  } as unknown as Api;
  function Harness() {
    const [workspaces, setWorkspaces] = useState(start);
    return (
      <SlackPanel
        api={api}
        fields={{ clientId: "", clientSecret: "", redirectUri: "" }}
        onFieldsChange={() => undefined}
        secretSaved={false}
        workspaces={workspaces}
        onWorkspacesChange={setWorkspaces}
      />
    );
  }
  render(<Harness />);
  const names = () => within(screen.getByRole("list", { name: "Connected workspaces" })).getAllByRole("listitem").map((li) => li.getAttribute("aria-label"));
  return { calls, names };
}

test("workspaces are listed in priority order", () => {
  const { names } = setup();
  expect(names()).toEqual(["Acme", "Acme Labs"]);
});

test("the switch turns a workspace off and back on, keeping its sign-in", async () => {
  const { calls } = setup();
  const toggle = screen.getByRole("switch", { name: "Use Acme Labs" });
  expect(toggle).toBeChecked();
  fireEvent.click(toggle);
  await waitFor(() => expect(screen.getByRole("switch", { name: "Use Acme Labs" })).not.toBeChecked());
  expect(within(screen.getByRole("listitem", { name: "Acme Labs" })).getByText("Disabled")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Remove Acme Labs" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("switch", { name: "Use Acme Labs" }));
  await waitFor(() => expect(screen.getByRole("switch", { name: "Use Acme Labs" })).toBeChecked());
  expect(calls.enabled).toEqual([
    ["T0ACME0002", false],
    ["T0ACME0002", true],
  ]);
});

test("the arrows reorder and save the priority; the ends are disabled", async () => {
  const { calls, names } = setup();
  expect(screen.getByRole("button", { name: "Move Acme up" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Move Acme Labs down" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Move Acme Labs up" }));
  await waitFor(() => expect(names()).toEqual(["Acme Labs", "Acme"]));
  expect(calls.order).toEqual([["T0ACME0002", "T0ACME0001"]]);
});

test("dragging a row onto another reorders", async () => {
  const { calls, names } = setup();
  fireEvent.dragStart(screen.getByRole("listitem", { name: "Acme Labs" }));
  fireEvent.dragOver(screen.getByRole("listitem", { name: "Acme" }));
  fireEvent.drop(screen.getByRole("listitem", { name: "Acme" }));
  await waitFor(() => expect(names()).toEqual(["Acme Labs", "Acme"]));
  expect(calls.order).toEqual([["T0ACME0002", "T0ACME0001"]]);
});

test("a failed save puts the order back and says why", async () => {
  const { names } = setup(undefined, true);
  fireEvent.click(screen.getByRole("button", { name: "Move Acme Labs up" }));
  expect(await screen.findByText("unknown workspace")).toBeInTheDocument();
  expect(names()).toEqual(["Acme", "Acme Labs"]);
});

test("the switch and arrows explain themselves in a tooltip", () => {
  vi.useFakeTimers();
  setup();
  fireEvent.mouseEnter(screen.getByRole("switch", { name: "Use Acme" }));
  act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS));
  expect(screen.getByRole("switch", { name: "Use Acme" })).toHaveAccessibleDescription(/skips this workspace for search, sends, bumps and status reads/);
  expect(screen.getByRole("button", { name: "Move Acme Labs up" })).toHaveAccessibleDescription(/Try this workspace earlier/);
});

test("one workspace has no reorder controls", () => {
  setup([acme]);
  expect(screen.queryByRole("button", { name: /Move/ })).not.toBeInTheDocument();
  expect(screen.getByRole("switch", { name: "Use Acme" })).toBeInTheDocument();
});

test("rows show a priority number and a drag handle; a stale workspace gets a prominent Reconnect", () => {
  setup([acme, { ...labs, signed_in: false, needs_sign_in: true }]);
  expect(within(screen.getByRole("listitem", { name: "Acme" })).getByLabelText("Priority 1")).toHaveTextContent("1");
  expect(within(screen.getByRole("listitem", { name: "Acme Labs" })).getByLabelText("Priority 2")).toHaveTextContent("2");
  expect(screen.getAllByTestId("drag-handle")).toHaveLength(2);
  expect(screen.getByRole("listitem", { name: "Acme Labs" })).toHaveAttribute("data-state", "needs-sign-in");
  expect(screen.getByRole("button", { name: "Reconnect Acme Labs" })).toBeInTheDocument();
});

test("a disabled workspace stays listed and connected but marked never used", () => {
  setup([acme, { ...labs, enabled: false }]);
  const row = screen.getByRole("listitem", { name: "Acme Labs" });
  expect(row).toHaveAttribute("data-state", "disabled");
  expect(within(row).getByText(/never used/)).toBeInTheDocument();
  expect(within(row).getByText("Disabled")).toBeInTheDocument();
});

test("each workspace shows its grant check, and Refresh updates the list", async () => {
  vi.useFakeTimers();
  const checkedAt = new Date(Date.now() - 3 * 60 * 1000).toISOString();
  const stored: SlackWorkspace[] = [
    { ...acme, check: "connected" as const, checked_at: checkedAt },
    { ...labs, check: "expired" as const, signed_in: false, needs_sign_in: true, checked_at: checkedAt },
    {
      id: 3,
      team_name: "Acme Archive",
      domain: "archive",
      user_name: "ada",
      signed_in: false,
      needs_sign_in: true,
      team_id: "T0DEAD0001",
      enabled: true,
      priority: 2,
      check: "missing" as const,
      checked_at: checkedAt,
    },
    {
      id: 4,
      team_name: "Acme Edge",
      domain: "edge",
      user_name: "ada",
      signed_in: true,
      needs_sign_in: false,
      team_id: "T0EDGE0001",
      enabled: true,
      priority: 3,
      check: "failed" as const,
      checked_at: checkedAt,
      check_error: "connection refused",
    },
    { ...labs, id: 5, team_name: "Acme Quiet", domain: "quiet", team_id: "T0QUIET001", enabled: false, priority: 4, check: "connected" as const, checked_at: checkedAt },
  ];
  const refreshed = stored.map((item) => (item.id === 2 ? { ...item, check: "connected" as const, signed_in: true, needs_sign_in: false, team_name: "Acme Labs" } : item));
  let release: (rows: SlackWorkspace[]) => void = () => undefined;
  const pending = new Promise<SlackWorkspace[]>((resolve) => {
    release = resolve;
  });
  const refresh = vi.fn(() => pending);
  function Harness() {
    const [workspaces, setWorkspaces] = useState(stored);
    const api = {
      slackRefreshWorkspaces: () => refresh(),
      slackSetWorkspaceEnabled: () => Promise.resolve(workspaces),
      slackSetWorkspaceOrder: () => Promise.resolve(workspaces),
    } as unknown as Api;
    return (
      <SlackPanel
        api={api}
        fields={{ clientId: "111.222", clientSecret: "", redirectUri: "" }}
        onFieldsChange={() => undefined}
        secretSaved={false}
        workspaces={workspaces}
        onWorkspacesChange={setWorkspaces}
      />
    );
  }
  render(<Harness />);
  const labsRow = screen.getByRole("listitem", { name: "Acme Labs" });
  expect(labsRow).toHaveAttribute("data-state", "needs-sign-in");
  expect(within(labsRow).getByText("Token expired")).toBeInTheDocument();
  expect(within(labsRow).getByRole("button", { name: "Reconnect Acme Labs" })).toBeInTheDocument();
  const archive = screen.getByRole("listitem", { name: "Acme Archive" });
  expect(within(archive).getByText("Missing grant")).toBeInTheDocument();
  expect(within(archive).getByRole("button", { name: "Reconnect Acme Archive" })).toBeInTheDocument();
  const edge = screen.getByRole("listitem", { name: "Acme Edge" });
  expect(edge).toHaveAttribute("data-state", "check-failed");
  expect(within(edge).queryByRole("button", { name: /Reconnect/ })).not.toBeInTheDocument();
  fireEvent.mouseEnter(within(edge).getByText("Check failed"));
  act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS));
  expect(within(edge).getByText("Check failed")).toHaveAccessibleDescription(/connection refused/);
  const acmeRow = screen.getByRole("listitem", { name: "Acme" });
  expect(within(acmeRow).getByText("Connected")).toBeInTheDocument();
  fireEvent.mouseEnter(within(acmeRow).getByText("Connected"));
  act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS));
  expect(within(acmeRow).getByText("Connected")).toHaveAccessibleDescription(/checked 3m ago/);
  const quiet = screen.getByRole("listitem", { name: "Acme Quiet" });
  expect(quiet).toHaveAttribute("data-state", "disabled");
  expect(within(quiet).getByText("Disabled")).toBeInTheDocument();
  expect(within(quiet).queryByRole("button", { name: /Reconnect/ })).not.toBeInTheDocument();

  vi.useRealTimers();
  fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
  expect(screen.getByRole("button", { name: "Refresh" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Refresh" })).toHaveAttribute("aria-busy", "true");
  expect(refresh).toHaveBeenCalledTimes(1);
  await act(async () => {
    release(refreshed);
    await pending;
  });
  await waitFor(() => expect(screen.getByRole("button", { name: "Refresh" })).not.toBeDisabled());
  expect(within(screen.getByRole("listitem", { name: "Acme Labs" })).getByText("Connected")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Reconnect Acme Labs" })).not.toBeInTheDocument();
});

test("opening Slack settings refreshes a check older than 5 minutes and skips a fresh one", async () => {
  const refresh = vi.fn(() => Promise.resolve([acme]));
  function renderAt(checked_at: string) {
    const api = { slackRefreshWorkspaces: refresh } as unknown as Api;
    render(
      <SlackPanel
        api={api}
        fields={{ clientId: "", clientSecret: "", redirectUri: "" }}
        onFieldsChange={() => undefined}
        secretSaved={false}
        workspaces={[{ ...acme, check: "connected", checked_at }]}
        onWorkspacesChange={() => undefined}
      />,
    );
  }
  renderAt(new Date().toISOString());
  await act(async () => {
    await Promise.resolve();
  });
  expect(refresh).not.toHaveBeenCalled();
  cleanup();
  renderAt(new Date(Date.now() - 6 * 60 * 1000).toISOString());
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
});
