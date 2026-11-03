import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { ProfileDto } from "../api";
import { AccountMenu, initials } from "./AccountMenu";

const profiles: ProfileDto[] = [
  { id: "default", name: "Work", active: true, githubLogin: "ada", isDefault: true },
  { id: "acme-labs", name: "Acme Labs", active: false, githubLogin: null, isDefault: false },
];

function setup() {
  const handlers = { onSwitch: vi.fn(), onAdd: vi.fn(), onSettings: vi.fn(), onAbout: vi.fn() };
  render(<AccountMenu profiles={profiles} settingsActive={false} {...handlers} />);
  return handlers;
}

test("initials fall back to the profile name", () => {
  expect(initials("Acme Labs")).toBe("AL");
  expect(initials("work")).toBe("W");
});

test("shows the active profile and opens a menu with every profile checked or not", () => {
  setup();
  const trigger = screen.getByRole("button", { name: "Profile Work, account menu" });
  expect(trigger).toHaveAttribute("aria-expanded", "false");
  fireEvent.click(trigger);
  expect(screen.getByRole("menuitemradio", { name: /Work/ })).toHaveAttribute("aria-checked", "true");
  expect(screen.getByRole("menuitemradio", { name: /Acme Labs/ })).toHaveAttribute("aria-checked", "false");
  expect(screen.getByRole("menuitemradio", { name: /Work/ })).toHaveFocus();
  expect(screen.getByRole("menuitemradio", { name: /Acme Labs/ }).className).toContain("cursor-pointer");
  // No GitHub account: initials instead of an avatar.
  expect(screen.getByText("AL")).toBeInTheDocument();
});

test("clicking another profile switches; the active one does nothing", () => {
  const handlers = setup();
  fireEvent.click(screen.getByRole("button", { name: /account menu/ }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /Work/ }));
  expect(handlers.onSwitch).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /account menu/ }));
  fireEvent.click(screen.getByRole("menuitemradio", { name: /Acme Labs/ }));
  expect(handlers.onSwitch).toHaveBeenCalledWith("acme-labs");
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});

test("Add a profile, Settings and About run their actions", () => {
  const handlers = setup();
  for (const [name, handler] of [
    ["Add a profile", handlers.onAdd],
    ["Settings", handlers.onSettings],
    ["About & what's new", handlers.onAbout],
  ] as const) {
    fireEvent.click(screen.getByRole("button", { name: /account menu/ }));
    fireEvent.click(screen.getByRole("menuitem", { name }));
    expect(handler).toHaveBeenCalledOnce();
  }
});

test("arrow keys move, Escape closes and refocuses the trigger, an outside click closes", () => {
  setup();
  const trigger = screen.getByRole("button", { name: /account menu/ });
  fireEvent.keyDown(trigger, { key: "ArrowUp" });
  const menu = screen.getByRole("menu");
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  expect(screen.getByRole("menuitemradio", { name: /Acme Labs/ })).toHaveFocus();
  fireEvent.keyDown(menu, { key: "End" });
  expect(screen.getByRole("menuitem", { name: "About & what's new" })).toHaveFocus();
  fireEvent.keyDown(menu, { key: "Escape" });
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();
  fireEvent.click(trigger);
  fireEvent.mouseDown(document.body);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});
