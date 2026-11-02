import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { ChangelogRelease } from "./changelog";
import { lastSeenVersion, recordSeenVersion } from "./changelog";
import { ChangelogSettings } from "./ChangelogSettings";
import { WhatsNew } from "./WhatsNew";

const releases: ChangelogRelease[] = [
  { version: "0.2.0", date: "2026-12-01", sections: [{ kind: "Added", entries: ["Shiny thing"] }] },
  { version: "0.1.1", date: "2026-11-01", sections: [{ kind: "Fixed", entries: ["Old bug"] }] },
  { version: "0.1.0", date: "2026-10-01", sections: [{ kind: "Added", entries: ["First"] }] },
];

function api(version: string) {
  return {
    appInfo: () => Promise.resolve({ version, repository: "acme/pr-nexus", db_path: "", log_dir: "", keyring_in_memory: false, os: "linux" }),
    openExternal: vi.fn(() => Promise.resolve()),
  };
}

beforeEach(() => window.localStorage.clear());

it("shows releases since the last seen version, traps focus and closes on Escape", async () => {
  recordSeenVersion("0.1.0");
  render(<WhatsNew api={api("0.2.0")} releases={releases} onFullChangelog={() => undefined} />);
  const dialog = await screen.findByRole("dialog", { name: /what's new in pr nexus v0\.2\.0/i });
  expect(dialog).toHaveTextContent("Shiny thing");
  expect(dialog).toHaveTextContent("Old bug");
  expect(dialog).not.toHaveTextContent("First");
  const gotIt = screen.getByRole("button", { name: "Got it" });
  expect(gotIt).toHaveFocus();
  fireEvent.keyDown(gotIt, { key: "Tab" });
  expect(screen.getByRole("button", { name: /close what's new/i })).toHaveFocus();
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(lastSeenVersion()).toBe("0.2.0");
});

it("records a fresh install without a modal", async () => {
  render(<WhatsNew api={api("0.2.0")} releases={releases} onFullChangelog={() => undefined} />);
  await waitFor(() => expect(lastSeenVersion()).toBe("0.2.0"));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("See full changelog dismisses and opens the tab", async () => {
  recordSeenVersion("0.1.1");
  const onFull = vi.fn();
  render(<WhatsNew api={api("0.2.0")} releases={releases} onFullChangelog={onFull} />);
  fireEvent.click(await screen.findByRole("button", { name: "See full changelog" }));
  expect(onFull).toHaveBeenCalled();
  expect(lastSeenVersion()).toBe("0.2.0");
});

it("Settings → Changelog lists releases, highlights the installed one and links GitHub releases", async () => {
  const fake = api("0.1.1");
  render(
    <ChangelogSettings
      api={fake}
      releases={releases}
      unreleased={{ version: null, date: null, sections: [{ kind: "Added", entries: ["Coming soon"] }] }}
    />,
  );
  expect(screen.getByRole("heading", { name: "Unreleased" })).toBeInTheDocument();
  await waitFor(() => expect(screen.getByRole("region", { name: "v0.1.1" })).toHaveAttribute("data-current", "true"));
  const headings = screen.getAllByRole("heading", { level: 3 }).map((node) => node.textContent);
  expect(headings).toEqual(["Unreleased", "v0.2.0", "v0.1.1", "v0.1.0"]);
  fireEvent.click(screen.getByRole("link", { name: /all releases on github/i }));
  expect(fake.openExternal).toHaveBeenCalledWith("https://github.com/acme/pr-nexus/releases");
});
