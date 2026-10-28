import { expect, test } from "vitest";
import { displayKey, groupedShortcuts, hint, matchShortcut, SHORTCUTS } from "./shortcuts";

const press = (key: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) =>
  matchShortcut({ key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods });

test("every binding resolves to its registry entry", () => {
  expect(press("j")).toBe("next");
  expect(press("ArrowDown")).toBe("next");
  expect(press("k")).toBe("prev");
  expect(press("x")).toBe("toggleRow");
  expect(press("X", { shiftKey: true })).toBe("toggleSection");
  expect(press("r")).toBe("request");
  expect(press("s")).toBe("slack");
  expect(press("S", { shiftKey: true })).toBe("star");
  expect(press("o")).toBe("github");
  expect(press("f")).toBe("filters");
  expect(press("p")).toBe("project");
  expect(press("/")).toBe("search");
  expect(press("?", { shiftKey: true })).toBe("help");
  expect(press("Enter")).toBe("details");
  expect(press("Escape")).toBe("clear");
  expect(press("a", { metaKey: true })).toBe("selectAll");
  expect(press("a", { ctrlKey: true })).toBe("selectAll");
  expect(press("k", { metaKey: true })).toBe("palette");
});

test("system shortcuts and unbound keys pass through", () => {
  expect(press("r", { metaKey: true })).toBeNull();
  expect(press("f", { ctrlKey: true })).toBeNull();
  expect(press("j", { altKey: true })).toBeNull();
  expect(press("J", { shiftKey: true })).toBeNull();
  expect(press("a")).toBeNull();
  expect(press("q")).toBeNull();
});

test("ids are unique and every group has shortcuts", () => {
  expect(new Set(SHORTCUTS.map((item) => item.id)).size).toBe(SHORTCUTS.length);
  for (const { items } of groupedShortcuts()) {
    expect(items.length).toBeGreaterThan(0);
  }
});

test("hints and display keys follow the platform", () => {
  expect(hint("request")).toBe("r");
  expect(displayKey("Mod+K", true)).toBe("⌘K");
  expect(displayKey("Mod+K", false)).toBe("Ctrl+K");
});
