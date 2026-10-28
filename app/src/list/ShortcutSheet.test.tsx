import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { ShortcutSheet } from "./ShortcutSheet";
import { displayKey, groupedShortcuts, SHORTCUTS } from "./shortcuts";

test("the cheat sheet lists every registry shortcut under its group", () => {
  render(<ShortcutSheet onClose={() => undefined} />);
  const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
  for (const { group, items } of groupedShortcuts()) {
    const section = within(dialog).getByRole("region", { name: group });
    for (const item of items) {
      const label = within(section).getByText(item.label);
      const keys = label.parentElement as HTMLElement;
      for (const key of item.keys) {
        expect(within(keys).getByText(displayKey(key))).toBeInTheDocument();
      }
    }
  }
  expect(dialog.querySelectorAll("dt")).toHaveLength(SHORTCUTS.length);
});

test("Esc closes the cheat sheet", () => {
  const onClose = vi.fn();
  render(<ShortcutSheet onClose={onClose} />);
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onClose).toHaveBeenCalled();
});
