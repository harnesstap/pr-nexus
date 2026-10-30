import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Api, UpdateInfo } from "../api";
import { UPDATE_CHECK_INTERVAL_MS, UpdateIndicator } from "./UpdateIndicator";

const release: UpdateInfo = {
  kind: "release",
  current: "0.1.0",
  version: "0.2.0",
  url: "https://github.com/acme/widgets/releases/tag/v0.2.0",
  command: "gh release download v0.2.0 -R acme/widgets -p '*.dmg'",
  behind_by: null,
};
const dev: UpdateInfo = {
  kind: "dev",
  current: "1111111",
  version: "2222222",
  url: "https://github.com/acme/widgets/compare/1111111...2222222",
  command: "git pull && bun install --cwd app && bun run tauri dev",
  behind_by: 3,
};

function fakeApi(result: () => Promise<UpdateInfo | null>, opened: string[] = []) {
  return {
    checkUpdate: vi.fn(result),
    openExternal: (url: string) => {
      opened.push(url);
      return Promise.resolve();
    },
  } as unknown as Api & { checkUpdate: ReturnType<typeof vi.fn> };
}

beforeEach(() => window.localStorage.clear());
afterEach(() => vi.useRealTimers());

test("up to date shows no icon", async () => {
  const api = fakeApi(() => Promise.resolve(null));
  const { container } = render(<UpdateIndicator api={api} />);
  await act(() => Promise.resolve());
  expect(api.checkUpdate).toHaveBeenCalledTimes(1);
  expect(container).toBeEmptyDOMElement();
});

test("a failed check shows no icon", async () => {
  const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
  const api = fakeApi(() => Promise.reject(new Error("offline")));
  const { container } = render(<UpdateIndicator api={api} />);
  await act(() => Promise.resolve());
  expect(container).toBeEmptyDOMElement();
  warn.mockRestore();
});

test("a newer release shows the icon, opens the release page and can be skipped", async () => {
  const opened: string[] = [];
  const api = fakeApi(() => Promise.resolve(release), opened);
  const { unmount } = render(<UpdateIndicator api={api} />);
  const icon = await screen.findByRole("button", { name: "PR Nexus 0.2.0 is available, update details" });
  fireEvent.click(icon);
  const dialog = screen.getByRole("dialog", { name: "Update available" });
  expect(dialog).toHaveTextContent("You have 0.1.0 · latest is 0.2.0");
  expect(dialog).not.toHaveTextContent("git pull");
  fireEvent.click(screen.getByRole("button", { name: /Open release page/ }));
  expect(opened).toEqual([release.url]);
  fireEvent.click(screen.getByRole("button", { name: "Skip this version" }));
  expect(screen.queryByRole("button", { name: /update details/ })).not.toBeInTheDocument();
  unmount();

  // Skipped stays skipped across launches, until a newer version shows up.
  render(<UpdateIndicator api={api} />);
  await act(() => Promise.resolve());
  expect(screen.queryByRole("button", { name: /update details/ })).not.toBeInTheDocument();
  render(<UpdateIndicator api={fakeApi(() => Promise.resolve({ ...release, version: "0.3.0" }))} />);
  expect(await screen.findByRole("button", { name: /0.3.0 is available/ })).toBeInTheDocument();
});

test("a dev build behind main shows the commit count and the rebuild command", async () => {
  const api = fakeApi(() => Promise.resolve(dev));
  render(<UpdateIndicator api={api} />);
  fireEvent.click(await screen.findByRole("button", { name: "3 commits behind main, update details" }));
  const dialog = screen.getByRole("dialog", { name: "Update available" });
  expect(dialog).toHaveTextContent("Built from 1111111 · main is at 2222222");
  expect(dialog).toHaveTextContent("git pull && bun install --cwd app && bun run tauri dev");
  expect(screen.getByRole("button", { name: "Skip this commit" })).toBeInTheDocument();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("checks again every six hours", async () => {
  vi.useFakeTimers();
  let answer: UpdateInfo | null = null;
  const api = fakeApi(() => Promise.resolve(answer));
  render(<UpdateIndicator api={api} />);
  await act(() => Promise.resolve());
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  answer = release;
  await act(async () => {
    vi.advanceTimersByTime(UPDATE_CHECK_INTERVAL_MS);
    await Promise.resolve();
  });
  expect(api.checkUpdate).toHaveBeenCalledTimes(2);
  expect(screen.getByRole("button", { name: /0.2.0 is available/ })).toBeInTheDocument();
});
