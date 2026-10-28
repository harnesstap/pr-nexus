import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { JiraLinkDto, ProjectDto } from "../api";
import { JiraChip, ProjectInfo } from "./JiraChips";
import { GitHubRowButton, StarButton } from "./StarButton";
import { StarredChip } from "./QuickFilters";

test("the star toggles without reaching the row underneath", () => {
  const onToggle = vi.fn();
  const onRow = vi.fn();
  render(
    <div onClick={onRow}>
      <StarButton starred={false} title="Migrate login" onToggle={onToggle} />
    </div>,
  );
  const star = screen.getByRole("button", { name: "Star Migrate login" });
  expect(star).toHaveAttribute("aria-pressed", "false");
  fireEvent.click(star);
  expect(onToggle).toHaveBeenCalledTimes(1);
  expect(onRow).not.toHaveBeenCalled();
});

test("a starred PR offers Unstar", () => {
  render(<StarButton starred title="Migrate login" onToggle={() => undefined} />);
  expect(screen.getByRole("button", { name: "Unstar Migrate login" })).toHaveAttribute("aria-pressed", "true");
});

const linked: JiraLinkDto = { id: 1, key: "ACME-123", url: "", href: "https://acme-labs.atlassian.net/browse/ACME-123" };
const bare: JiraLinkDto = { id: 2, key: "ACME-7", url: "", href: null };

test("a JIRA chip links out when it has an href and is plain text otherwise", () => {
  const onOpen = vi.fn();
  render(
    <>
      <JiraChip link={linked} onOpen={onOpen} />
      <JiraChip link={bare} onOpen={onOpen} />
    </>,
  );
  fireEvent.click(screen.getByRole("link", { name: "ACME-123" }));
  expect(onOpen).toHaveBeenCalledWith("https://acme-labs.atlassian.net/browse/ACME-123");
  expect(screen.getByText("ACME-7").closest("a")).toBeNull();
});

test("project info shows chips and the first description line, and nothing when empty", () => {
  const project: ProjectDto = { id: 1, name: "Auth", color: "#7c5cbf", archived: false, description: "Login revamp\nSecond line", sort_order: 0, jira_links: [linked] };
  const { rerender, container } = render(<ProjectInfo project={project} />);
  expect(screen.getByText("Login revamp")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "ACME-123" })).toBeInTheDocument();
  rerender(<ProjectInfo project={{ ...project, description: " ", jira_links: [] }} />);
  expect(container).toBeEmptyDOMElement();
});

test("the row GitHub button opens the PR without bubbling to the row", () => {
  const onOpen = vi.fn();
  const onRow = vi.fn();
  render(
    <div onClick={onRow}>
      <GitHubRowButton title="Fix tax rounding" onOpen={onOpen} />
    </div>,
  );
  const button = screen.getByRole("button", { name: "Open Fix tax rounding on GitHub" });
  expect(button.getAttribute("title")).toBeNull();
  fireEvent.click(button);
  expect(onOpen).toHaveBeenCalledOnce();
  expect(onRow).not.toHaveBeenCalled();
});

test("the Starred chip shows its count and toggles", () => {
  const onChange = vi.fn();
  render(<StarredChip on={false} count={3} onChange={onChange} />);
  fireEvent.click(screen.getByRole("button", { name: "Starred only (3)" }));
  expect(onChange).toHaveBeenCalledWith(true);
});
