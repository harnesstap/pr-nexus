import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, ListRow, ProjectDto } from "../api";
import { memoryProjectApi } from "../memoryProjects";
import { ProjectDialog } from "./ProjectDialog";

const project: ProjectDto = { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "", sort_order: 0 };

function row(id: number, title: string, project_ids: number[]): ListRow {
  return {
    id,
    number: 100 + id,
    title,
    url: `https://github.com/acme/widgets/pull/${100 + id}`,
    body: "",
    owner: "acme",
    repo: "acme/widgets",
    author_login: "ada",
    is_draft: false,
    state: "open",
    checks: "success",
    review_decision: null,
    unresolved_threads: 0,
    total_threads: 0,
    involvement: 1,
    updated_at: "2026-10-26T11:00:00Z",
    created_at: "2026-10-20T11:00:00Z",
    hidden: false,
    project_ids,
    readiness: "ready",
    readiness_notes: [],
    slack: null,
  };
}

function setup() {
  const rows = [row(1, "Migrate login", [1]), row(2, "Add passkeys", []), row(3, "Fix tax rounding", [])];
  const memory = memoryProjectApi(rows, [project]);
  const api = { ...memory, previewRule: vi.fn(() => Promise.resolve(2)), saveProjectRule: vi.fn(memory.saveProjectRule) } as unknown as Api;
  const onChanged = vi.fn(() => Promise.resolve());
  const onClose = vi.fn();
  render(<ProjectDialog api={api} project={project} rows={rows} onClose={onClose} onChanged={onChanged} />);
  return { api, onChanged, onClose };
}

test("rules get a live preview and save with their kind, pattern and priority", async () => {
  const { api, onChanged } = setup();
  const dialog = screen.getByRole("dialog", { name: "Auth" });
  expect(await within(dialog).findByText(/No rules/)).toBeInTheDocument();
  fireEvent.click(within(dialog).getByRole("button", { name: "Add rule" }));
  fireEvent.change(within(dialog).getByRole("combobox", { name: "Rule kind" }), { target: { value: "title_regex" } });
  fireEvent.change(within(dialog).getByRole("textbox", { name: "Rule pattern" }), { target: { value: "(?i)login" } });
  fireEvent.change(within(dialog).getByRole("spinbutton", { name: "Rule priority" }), { target: { value: "5" } });
  expect(await within(dialog).findByText("Matches 2 open PRs")).toBeInTheDocument();
  expect(api.previewRule).toHaveBeenLastCalledWith("title_regex", "(?i)login");
  fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(api.saveProjectRule).toHaveBeenCalledWith({ id: null, project_id: 1, kind: "title_regex", pattern: "(?i)login", priority: 5 }),
  );
  await waitFor(() => expect(onChanged).toHaveBeenCalled());
});

test("PRs are added from the picker, removed, and the dialog closes on Escape", async () => {
  const { onChanged, onClose } = setup();
  const dialog = screen.getByRole("dialog", { name: "Auth" });
  expect(await within(dialog).findByText("Linked pull requests (1)")).toBeInTheDocument();
  expect(within(dialog).getByText("Manual")).toBeInTheDocument();
  fireEvent.change(within(dialog).getByRole("textbox", { name: "Add pull request" }), { target: { value: "passkeys" } });
  const matches = within(dialog).getByRole("list", { name: "Matching pull requests" });
  expect(within(matches).queryByText(/Migrate login/)).not.toBeInTheDocument();
  fireEvent.click(within(matches).getByRole("button", { name: "Add Add passkeys" }));
  expect(await within(dialog).findByText("Linked pull requests (2)")).toBeInTheDocument();
  fireEvent.click(within(dialog).getByRole("button", { name: "Remove Migrate login" }));
  expect(await within(dialog).findByText("Linked pull requests (1)")).toBeInTheDocument();
  expect(onChanged).toHaveBeenCalledTimes(2);
  await act(async () => undefined);
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalled();
});

test("a rule-linked PR that is removed becomes excluded and can be restored", async () => {
  const rows = [row(1, "Migrate login", [])];
  const memory = memoryProjectApi(rows, [project]);
  await memory.linkPrs(1, [1]);
  const api = {
    ...memory,
    listProjectLinks: vi
      .fn()
      .mockResolvedValueOnce([{ pr_id: 1, source: "rule", rule_id: 7 }])
      .mockResolvedValueOnce([{ pr_id: 1, source: "excluded", rule_id: null }])
      .mockResolvedValue([{ pr_id: 1, source: "rule", rule_id: 7 }]),
    listProjectRules: () => Promise.resolve([{ id: 7, project_id: 1, kind: "ticket_key_in_branch", pattern: "AUTH-*", priority: 0 }]),
    unlinkPr: vi.fn(() => Promise.resolve()),
    restorePr: vi.fn(() => Promise.resolve()),
  } as unknown as Api;
  render(<ProjectDialog api={api} project={project} rows={rows} onClose={() => undefined} onChanged={() => Promise.resolve()} />);
  const badge = await screen.findByText("Rule");
  expect(badge).toHaveAttribute("title", "Ticket key: AUTH-*");
  fireEvent.click(screen.getByRole("button", { name: "Remove Migrate login" }));
  expect(await screen.findByText("Excluded")).toBeInTheDocument();
  expect(api.unlinkPr).toHaveBeenCalledWith(1, 1);
  fireEvent.click(screen.getByRole("button", { name: "Restore Migrate login" }));
  expect(await screen.findByText("Rule")).toBeInTheDocument();
  expect(api.restorePr).toHaveBeenCalledWith(1, 1);
});
