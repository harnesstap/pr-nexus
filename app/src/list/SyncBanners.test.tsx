import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import type { SlackWorkspace, SyncStatus } from "../api";
import { SlackSignInBanner, SyncErrorBanner, SyncPopover, slackActionBlock, syncProblem, workspacesNeedingSignIn } from "./SyncBanners";

const now = new Date("2026-10-27T12:00:00Z");
const ok: SyncStatus = { phase: "ok", message: "Synced", synced_at: "2026-10-27T09:00:00Z", paused: false, revision: 1 };
const failed = (message: string): SyncStatus => ({ ...ok, phase: "error", message: `Sync failed · ${message}` });
const acme: SlackWorkspace = { id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 };
const labs: SlackWorkspace = { id: 2, team_name: "Acme Labs", domain: "acme-labs", user_name: "ada", signed_in: false, needs_sign_in: true, team_id: "T0ACME0002", enabled: true, priority: 1 };

test("no banner while sync is fine, paused or syncing", () => {
  for (const phase of ["ok", "idle", "syncing", "paused", "demo"]) {
    expect(syncProblem({ ...ok, phase }, now)).toBeNull();
  }
});

test("a 401 is an auth problem with the data age", () => {
  expect(syncProblem(failed("GitHub returned 401 Bad credentials"), now)).toMatchObject({ kind: "auth", age: "data from 3h ago" });
  expect(syncProblem(failed("unauthorized: token expired"), now)?.kind).toBe("auth");
});

test("other errors are plain errors, and no age before the first sync", () => {
  expect(syncProblem({ ...failed("network unreachable"), synced_at: null }, now)).toMatchObject({ kind: "error", age: null });
});

test("the auth banner offers Reconnect GitHub, other errors Try again", () => {
  const calls: string[] = [];
  const { rerender } = render(<SyncErrorBanner sync={failed("401")} now={now} onReconnect={() => calls.push("reconnect")} onRetry={() => calls.push("retry")} />);
  expect(screen.getByRole("alert")).toHaveTextContent(/data from 3h ago/);
  fireEvent.click(screen.getByRole("button", { name: "Reconnect GitHub" }));
  rerender(<SyncErrorBanner sync={failed("timeout")} now={now} onReconnect={() => calls.push("reconnect")} onRetry={() => calls.push("retry")} />);
  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  expect(calls).toEqual(["reconnect", "retry"]);
  rerender(<SyncErrorBanner sync={ok} now={now} onReconnect={() => undefined} onRetry={() => undefined} />);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("only enabled workspaces that need sign-in warn", () => {
  expect(workspacesNeedingSignIn([acme, labs]).map((item) => item.team_id)).toEqual(["T0ACME0002"]);
  expect(workspacesNeedingSignIn([acme, { ...labs, enabled: false }])).toEqual([]);
});

test("the Slack banner names the workspace and reconnects", () => {
  let opened = 0;
  render(<SlackSignInBanner workspaces={[acme, labs]} onReconnect={() => (opened += 1)} />);
  expect(screen.getByRole("alert")).toHaveTextContent("Acme Labs Slack needs sign-in");
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  expect(opened).toBe(1);
});

test("Bump is blocked for a thread in a workspace needing sign-in; a new request only when all are", () => {
  expect(slackActionBlock([acme, labs], "acme-labs")).toBe("Acme Labs Slack needs sign-in");
  expect(slackActionBlock([acme, labs], "acme")).toBeNull();
  expect(slackActionBlock([acme, labs])).toBeNull();
  expect(slackActionBlock([{ ...acme, enabled: false }, labs])).toBe("Acme Labs Slack needs sign-in");
  expect(slackActionBlock([])).toBeNull();
});

test("the sync popover has labeled rows, real buttons, and closes on Escape", () => {
  const calls: string[] = [];
  render(
    <SyncPopover
      sync={ok}
      now={now}
      prCount={42}
      onSync={() => calls.push("sync")}
      onFullResync={() => calls.push("full")}
      onClose={() => calls.push("close")}
    />,
  );
  const dialog = screen.getByRole("dialog", { name: "Sync details" });
  for (const label of ["Status", "Last sync", "Live updates", "Open PRs"]) {
    expect(dialog).toHaveTextContent(label);
  }
  expect(dialog).toHaveTextContent("42");
  fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
  fireEvent.click(screen.getByRole("button", { name: "Full resync" }));
  fireEvent.keyDown(document, { key: "Escape" });
  expect(calls).toEqual(["sync", "full", "close"]);
});
