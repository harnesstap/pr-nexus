import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { QuickFilters } from "./QuickFilters";

const counts = { draft: 2, no_review: 1, ci_trouble: 3, declined: 0, ready: 4 };

function setup(props: Partial<Parameters<typeof QuickFilters>[0]> = {}) {
  const onChange = vi.fn();
  const onStarredChange = vi.fn();
  render(<QuickFilters active={null} counts={counts} onChange={onChange} starred={false} starredCount={5} onStarredChange={onStarredChange} {...props} />);
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
  expect(screen.queryByRole("button", { name: "Clear quick filters" })).toBeNull();
});

test("clicking a chip picks it, clicking the active one clears it", () => {
  const { onChange } = setup({ active: "ready" });
  const ready = screen.getByRole("button", { name: "Ready (4)" });
  expect(ready).toHaveAttribute("aria-pressed", "true");
  expect(ready.className).toContain("border-emerald-400");
  fireEvent.click(ready);
  expect(onChange).toHaveBeenCalledWith(null);
  fireEvent.click(screen.getByRole("button", { name: "Draft (2)" }));
  expect(onChange).toHaveBeenLastCalledWith("draft");
});

test("Clear turns off the quick filter and Starred together", () => {
  const { onChange, onStarredChange } = setup({ active: "draft", starred: true });
  fireEvent.click(screen.getByRole("button", { name: "Clear quick filters" }));
  expect(onChange).toHaveBeenCalledWith(null);
  expect(onStarredChange).toHaveBeenCalledWith(false);
});

test("later chips fold into More on a narrow list, but an active one stays out", () => {
  setup({ active: "ready" });
  expect(screen.getByRole("button", { name: "Changes requested (0)" })).toHaveClass("hidden", "@xl:flex");
  expect(screen.getByRole("button", { name: "Ready (4)" })).not.toHaveClass("hidden");
  expect(screen.getByRole("button", { name: "Draft (2)" })).not.toHaveClass("hidden");
  fireEvent.click(screen.getByRole("button", { name: "More quick filters (2)" }));
  const menu = screen.getByRole("menu", { name: "More quick filters" });
  expect(within(menu).getAllByRole("menuitemcheckbox").map((item) => item.textContent)).toEqual(["Changes requested0", "Starred5"]);
});

test("picking from More applies that filter and closes the menu", () => {
  const { onStarredChange } = setup();
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
