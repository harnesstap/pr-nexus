import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { SelectionBar, barLayout } from "./SelectionBar";

afterEach(() => vi.restoreAllMocks());

function bar(
  width: number,
  props: Partial<Parameters<typeof SelectionBar>[0]> = {},
) {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(width);
  const handlers = {
    onRequest: vi.fn(),
    onCopy: vi.fn(),
    onClear: vi.fn(),
    onClose: vi.fn(),
    onToggleAddMenu: vi.fn(),
    onMarkRead: vi.fn(),
    onMarkAllRead: vi.fn(),
    onSnooze: vi.fn(),
    onMute: vi.fn(),
  };
  render(
    <SelectionBar
      count={5}
      hidden={0}
      addMenuOpen={false}
      addMenu={<div role="dialog" aria-label="Projects" />}
      {...handlers}
      {...props}
    />,
  );
  return {
    region: screen.getByRole("region", { name: "Selection actions" }),
    ...handlers,
  };
}

test("the layout steps down as the list column narrows", () => {
  expect(barLayout(0)).toBe("full");
  expect(barLayout(1200)).toBe("full");
  expect(barLayout(600)).toBe("more");
  expect(barLayout(400)).toBe("compact");
});

test("the bar is laid out inside the list column, never wider than it", () => {
  const { region } = bar(600);
  const host = screen.getByTestId("selection-bar-host");
  expect(host.className).toContain("inset-x-0");
  expect(host.className).toContain("absolute");
  expect(region.className).toContain("max-w-full");
});

test("a wide column keeps Add and Copy inline and puts Close in More", () => {
  const { region, onClose } = bar(1000);
  expect(region.getAttribute("data-layout")).toBe("full");
  expect(
    within(region).getByRole("button", { name: /Add to project/ }),
  ).toBeTruthy();
  expect(
    within(region).getByRole("button", { name: /Copy as Markdown/ }),
  ).toBeTruthy();
  fireEvent.click(within(region).getByRole("button", { name: "More actions" }));
  const items = within(region).getAllByRole("menuitem");
  expect(items.map((item) => item.textContent)).toEqual([
    "Mark as read",
    "Mark all as read",
    "Snooze for 1 day",
    "Snooze for 1 week",
    "Snooze until update",
    "Unsnooze",
    "Mute",
    "Unmute",
    "Close 5 PRs",
  ]);
  fireEvent.click(within(region).getByRole("menuitem", { name: "Close 5 PRs" }));
  expect(onClose).toHaveBeenCalledTimes(1);
  expect(region.textContent).toContain("5 selected");
});

test("the More menu holds the folded actions and fires them", () => {
  const { region, onCopy, onToggleAddMenu } = bar(600);
  fireEvent.click(within(region).getByRole("button", { name: "More actions" }));
  const menu = within(region).getByRole("menu");
  const items = within(menu).getAllByRole("menuitem");
  expect(items.map((item) => item.textContent)).toEqual([
    expect.stringContaining("Add to project"),
    expect.stringContaining("Copy as Markdown"),
    "Mark as read",
    "Mark all as read",
    "Snooze for 1 day",
    "Snooze for 1 week",
    "Snooze until update",
    "Unsnooze",
    "Mute",
    "Unmute",
    "Close 5 PRs",
  ]);
  expect(document.activeElement).toBe(items[0]);
  fireEvent.keyDown(items[0], { key: "ArrowDown" });
  expect(document.activeElement).toBe(items[1]);
  fireEvent.click(items[1]);
  expect(onCopy).toHaveBeenCalledTimes(1);
  expect(within(region).queryByRole("menu")).toBeNull();
  fireEvent.click(within(region).getByRole("button", { name: "More actions" }));
  fireEvent.click(
    within(region).getByRole("menuitem", { name: /Add to project/ }),
  );
  expect(onToggleAddMenu).toHaveBeenCalledTimes(1);
});

test("Escape closes the More menu and returns focus to its button", () => {
  const { region } = bar(600);
  const more = within(region).getByRole("button", { name: "More actions" });
  fireEvent.click(more);
  fireEvent.keyDown(within(region).getAllByRole("menuitem")[0], {
    key: "Escape",
  });
  expect(within(region).queryByRole("menu")).toBeNull();
  expect(document.activeElement).toBe(more);
});

test("the Add to project popover anchors to More when folded", () => {
  const { region } = bar(600, { addMenuOpen: true });
  expect(within(region).getByRole("dialog", { name: "Projects" })).toBeTruthy();
});

test("a very narrow column shortens the count and the Slack button but keeps it and Clear", () => {
  const { region, onRequest, onClear } = bar(400, { hidden: 2 });
  expect(region.getAttribute("data-layout")).toBe("compact");
  expect(region.textContent).toContain("5 (+2 hidden)");
  const slack = within(region).getByRole("button", {
    name: "Request / bump on Slack",
  });
  expect(slack.textContent).toBe("Slack");
  fireEvent.click(slack);
  fireEvent.click(
    within(region).getByRole("button", { name: "Clear selection" }),
  );
  expect(onRequest).toHaveBeenCalled();
  expect(onClear).toHaveBeenCalled();
});
