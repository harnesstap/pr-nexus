import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test } from "vitest";
import type { Api } from "../api";
import { UpdateBanner } from "./Banners";
import { showMappingHint, dismissMappingHint } from "./hints";

const update = { version: "0.2.0", url: "https://github.com/acme/pr-nexus/releases/tag/v0.2.0", command: "gh release download v0.2.0" };

beforeEach(() => window.localStorage.clear());

test("the update banner links the release and stays dismissed for that version only", async () => {
  const opened: string[] = [];
  const api = {
    checkUpdate: () => Promise.resolve(update),
    openExternal: (url: string) => {
      opened.push(url);
      return Promise.resolve();
    },
  } as unknown as Api;
  const { unmount } = render(<UpdateBanner api={api} />);
  expect(await screen.findByText("PR-Nexus 0.2.0 is available.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /release page/ }));
  expect(opened).toEqual([update.url]);
  fireEvent.click(screen.getByRole("button", { name: "Dismiss update notice" }));
  expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  unmount();

  render(<UpdateBanner api={api} />);
  await Promise.resolve();
  expect(screen.queryByText(/is available/)).not.toBeInTheDocument();

  const next = { checkUpdate: () => Promise.resolve({ ...update, version: "0.3.0" }), openExternal: () => Promise.resolve() } as unknown as Api;
  render(<UpdateBanner api={next} />);
  expect(await screen.findByText("PR-Nexus 0.3.0 is available.")).toBeInTheDocument();
});

test("a failed update check shows nothing", async () => {
  const api = { checkUpdate: () => Promise.reject(new Error("offline")) } as unknown as Api;
  const { container } = render(<UpdateBanner api={api} />);
  await Promise.resolve();
  expect(container).toBeEmptyDOMElement();
});

test("the mapping hint needs Slack, no rows, and no dismissal", () => {
  expect(showMappingHint(0, 1)).toBe(true);
  expect(showMappingHint(3, 1)).toBe(false);
  expect(showMappingHint(0, 0)).toBe(false);
  dismissMappingHint();
  expect(showMappingHint(0, 1)).toBe(false);
});
