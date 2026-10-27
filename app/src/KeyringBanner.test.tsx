import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { KeyringStatus } from "./api";
import { KeyringBanner } from "./KeyringBanner";

const waiting: KeyringStatus = {
  state: "waiting",
  message: "Waiting for keychain access. If a system prompt asks for PR Nexus's keychain item, approve it (on macOS, click Always Allow).",
  can_retry: false,
  unsaved: false,
};
const timedOut: KeyringStatus = {
  state: "timed_out",
  message: "Still waiting for keychain access after 5 minutes. Tokens stay in memory for now and are saved to the keychain as soon as it answers. Approve the system prompt, or retry.",
  can_retry: true,
  unsaved: true,
};
const denied: KeyringStatus = {
  state: "denied",
  message: "Keychain access was denied. Tokens stay in memory for this session and are lost when the app quits. Retry to ask again.",
  can_retry: true,
  unsaved: true,
};
const ready: KeyringStatus = { state: "ready", message: null, can_retry: false, unsaved: false };

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

async function tick(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

test("a pending prompt shows a hint, not a warning, and the late answer hides it", async () => {
  const answers = [waiting, timedOut, ready];
  const keyringStatus = vi.fn(() => Promise.resolve(answers.shift() ?? ready));
  const onReady = vi.fn();
  render(<KeyringBanner api={{ keyringStatus, keyringRetry: vi.fn() }} initial={waiting} onReady={onReady} pollMs={100} />);
  expect(screen.getByRole("status")).toHaveTextContent("Waiting for keychain access");
  expect(screen.queryByRole("button")).not.toBeInTheDocument();

  await tick(200);
  expect(screen.getByRole("alert")).toHaveTextContent("saved to the keychain as soon as it answers");
  expect(screen.getByRole("button", { name: "Retry keychain access" })).toBeInTheDocument();

  await tick(100);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  expect(onReady).toHaveBeenCalledTimes(1);
  const calls = keyringStatus.mock.calls.length;
  await tick(500);
  expect(keyringStatus.mock.calls.length).toBe(calls);
});

test("deny shows the reason with Retry, and Retry waits for the new prompt", async () => {
  const keyringStatus = vi.fn(() => Promise.resolve(ready));
  const keyringRetry = vi.fn(() => Promise.resolve(waiting));
  const onReady = vi.fn();
  render(<KeyringBanner api={{ keyringStatus, keyringRetry }} initial={denied} onReady={onReady} pollMs={100} />);
  expect(screen.getByRole("alert")).toHaveTextContent("Keychain access was denied");
  await tick(500);
  expect(keyringStatus).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: "Retry keychain access" }));
  await tick(0);
  expect(keyringRetry).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("status")).toHaveTextContent("Waiting for keychain access");
  await tick(100);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  expect(onReady).toHaveBeenCalledTimes(1);
});

test("without a bootstrap status it asks once and shows nothing when ready", async () => {
  const keyringStatus = vi.fn(() => Promise.resolve(ready));
  const { container } = render(<KeyringBanner api={{ keyringStatus, keyringRetry: vi.fn() }} pollMs={100} />);
  await tick(0);
  expect(keyringStatus).toHaveBeenCalledTimes(1);
  expect(container).toBeEmptyDOMElement();
  await tick(500);
  expect(keyringStatus).toHaveBeenCalledTimes(1);
});
