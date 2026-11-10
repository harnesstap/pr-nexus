import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { QuickFilters, fitChips } from "./QuickFilters";

const counts = { draft: 2, no_review: 1, ci_trouble: 3, declined: 0, ready: 4 };

function setup(props: Partial<Parameters<typeof QuickFilters>[0]> = {}) {
  const onChange = vi.fn();
  const onStarredChange = vi.fn();
  render(
    <QuickFilters
      active={null}
      counts={counts}
      onChange={onChange}
      starred={false}
      starredCount={5}
      onStarredChange={onStarredChange}
      unread={false}
      unreadCount={1}
      onUnreadChange={vi.fn()}
      snoozed={false}
      snoozedCount={2}
      onSnoozedChange={vi.fn()}
      {...props}
    />,
  );
  return { onChange, onStarredChange };
}

test("each chip shows its label and count, with a pointer cursor", () => {
  setup();
  for (const name of ["Draft (2)", "No review (1)", "CI or conflicts (3)", "Changes requested (0)", "Ready (4)", "Starred (5)"]) {
    const chip = screen.getByRole("button", { name });
    expect(chip).toHaveAttribute("aria-pressed", "false");
    expect(chip).toHaveClass("cursor-pointer");
    expect(chip).toHaveTextContent(name.replace(/ \((\d+)\)$/, "$1"));
  }
  expect(screen.queryByRole("button", { name: /Clear/ })).toBeNull();
});

test("clicking a chip picks it, clicking the active one clears it", () => {
  const { onChange } = setup({ active: "ready" });
  const ready = screen.getByRole("button", { name: "Ready (4)" });
  expect(ready).toHaveAttribute("aria-pressed", "true");
  expect(ready.className).toContain("bg-violet-500/15");
  fireEvent.click(ready);
  expect(onChange).toHaveBeenCalledWith(null);
  fireEvent.click(screen.getByRole("button", { name: "Draft (2)" }));
  expect(onChange).toHaveBeenLastCalledWith("draft");
});

test("a chip with nothing to show is muted but still clickable", () => {
  const { onChange } = setup();
  const declined = screen.getByRole("button", { name: "Changes requested (0)" });
  expect(declined).toHaveClass("opacity-60");
  expect(screen.getByRole("button", { name: "Draft (2)" })).not.toHaveClass("opacity-60");
  fireEvent.click(declined);
  expect(onChange).toHaveBeenCalledWith("declined");
});

test("fitChips keeps everything when it fits and folds the tail otherwise", () => {
  const keys = ["draft", "no_review", "ci_trouble", "declined", "ready", "starred"] as const;
  const widths = [50, 50, 50, 50, 50, 50];
  const off = () => false;
  expect([...fitChips(keys, widths, 320, 60, off)]).toEqual([...keys]);
  // 200 - More 60 - gap 4 = 136: two 54px chips fit.
  expect([...fitChips(keys, widths, 200, 60, off)]).toEqual(["draft", "no_review"]);
});

test("fitChips keeps active chips visible first", () => {
  const keys = ["draft", "no_review", "ci_trouble", "declined", "ready", "starred"] as const;
  const shown = fitChips(keys, [50, 50, 50, 50, 50, 50], 200, 60, (key) => key === "ready" || key === "starred");
  expect([...shown].sort()).toEqual(["ready", "starred"]);
});

test("chips that do not fit the measured width fold into More", () => {
  const width = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(60);
  const client = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(200);
  try {
    setup({ active: "ready" });
    expect(screen.getByRole("button", { name: "Ready (4)" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Draft (2)" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Starred (5)" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "More quick filters (6)" }));
    const menu = screen.getByRole("menu", { name: "More quick filters" });
    expect(within(menu).getAllByRole("menuitemcheckbox").map((item) => item.textContent)).toEqual([
      "No review1",
      "CI or conflicts3",
      "Changes requested0",
      "Starred5",
      "Unread1",
      "Snoozed2",
    ]);
  } finally {
    width.mockRestore();
    client.mockRestore();
  }
});

test("picking from More applies that filter and closes the menu", () => {
  const width = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(60);
  const client = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(200);
  const { onStarredChange } = setup();
  width.mockRestore();
  client.mockRestore();
  fireEvent.click(screen.getByRole("button", { name: /^More quick filters/ }));
  fireEvent.click(within(screen.getByRole("menu")).getByRole("menuitemcheckbox", { name: /Starred/ }));
  expect(onStarredChange).toHaveBeenCalledWith(true);
  expect(screen.queryByRole("menu")).toBeNull();
});

test("without counts yet the chips still render their labels", () => {
  setup({ counts: null, starredCount: null });
  expect(screen.getByRole("button", { name: "Draft" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Starred" })).toBeInTheDocument();
});
