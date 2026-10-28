import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { TOOLTIP_DELAY_MS, Tooltip, placeBubble } from "./Tooltip";

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

test("the open bubble is portaled to the body so overflow containers do not clip it", () => {
  render(
    <div style={{ overflow: "hidden" }} data-testid="clip">
      <Tooltip label="Requested 6h ago">
        <span>acme</span>
      </Tooltip>
    </div>,
  );
  const anchor = screen.getByText("acme");
  fireEvent.keyDown(document, { key: "Tab" });
  fireEvent.focus(anchor);
  const tip = screen.getByRole("tooltip");
  expect(screen.getByTestId("clip")).not.toContainElement(tip);
  expect(tip.parentElement).toBe(document.body);
  expect(anchor).toHaveAccessibleDescription("Requested 6h ago");
});

test("an empty label renders the control alone", () => {
  render(
    <Tooltip label={null}>
      <button type="button">Rename</button>
    </Tooltip>,
  );
  expect(screen.getByRole("button", { name: "Rename" })).not.toHaveAttribute("aria-describedby");
});

test("only one bubble is open at a time", () => {
  render(
    <>
      <Tooltip label="First">
        <button type="button">one</button>
      </Tooltip>
      <Tooltip label="Second">
        <button type="button">two</button>
      </Tooltip>
    </>,
  );
  fireEvent.keyDown(document, { key: "Tab" });
  fireEvent.focus(screen.getByRole("button", { name: "one" }));
  fireEvent.focus(screen.getByRole("button", { name: "two" }));
  expect(screen.getAllByRole("tooltip")).toHaveLength(1);
  expect(screen.getByRole("tooltip")).toHaveTextContent("Second");
});

test("placeBubble flips near the bottom edge and clamps to the window", () => {
  const view = { width: 400, height: 300 };
  const bubble = { width: 120, height: 40 };
  expect(placeBubble({ top: 100, bottom: 120, left: 100, right: 140 }, bubble, view, "bottom", "center")).toEqual({ top: 126, left: 60 });
  expect(placeBubble({ top: 270, bottom: 290, left: 100, right: 140 }, bubble, view, "bottom", "center").top).toBe(224);
  expect(placeBubble({ top: 2, bottom: 20, left: 100, right: 140 }, bubble, view, "top", "center").top).toBe(26);
  expect(placeBubble({ top: 100, bottom: 120, left: 380, right: 396 }, bubble, view, "bottom", "center").left).toBe(272);
  expect(placeBubble({ top: 100, bottom: 120, left: 0, right: 10 }, bubble, view, "bottom", "end").left).toBe(8);
});
