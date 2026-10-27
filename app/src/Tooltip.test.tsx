import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { TOOLTIP_DELAY_MS, Tooltip } from "./Tooltip";

afterEach(() => vi.useRealTimers());

function renderButton() {
  render(
    <Tooltip label="Sync now" detail="Fetch the latest PRs from GitHub" shortcut="⌘R">
      <button type="button" aria-label="Sync now">
        ↻
      </button>
    </Tooltip>,
  );
  return screen.getByRole("button", { name: "Sync now" });
}

test("shows the label and explanation after a hover delay, and hides on leave", () => {
  vi.useFakeTimers();
  const button = renderButton();
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  fireEvent.mouseEnter(button);
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS));
  const tip = screen.getByRole("tooltip");
  expect(tip).toHaveTextContent("Sync now⌘RFetch the latest PRs from GitHub");
  expect(button).toHaveAccessibleDescription("Sync now⌘RFetch the latest PRs from GitHub");
  fireEvent.mouseLeave(button);
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
});

test("keyboard focus shows it at once; Escape and blur hide it; a click does not open it", () => {
  const button = renderButton();
  fireEvent.keyDown(document, { key: "Tab" });
  fireEvent.focus(button);
  expect(screen.getByRole("tooltip")).toBeVisible();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  fireEvent.keyDown(document, { key: "Tab" });
  fireEvent.focus(button);
  fireEvent.blur(button);
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  fireEvent.pointerDown(button);
  fireEvent.focus(button);
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
});

test("focus moved by code after a click (a dialog opening) does not open it", () => {
  const button = renderButton();
  fireEvent.mouseDown(document.body);
  fireEvent.focus(button);
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
});
