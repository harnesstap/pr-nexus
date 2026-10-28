import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
  expect(within(screen.getByRole("listitem", { name: "Acme Labs" })).getByText(/disabled/)).toBeInTheDocument();
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
});
