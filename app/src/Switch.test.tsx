import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { expect, test } from "vitest";
import { Switch } from "./Switch";

function Harness({ disabled = false }: { disabled?: boolean }) {
  const [on, setOn] = useState(false);
  return (
    <Switch label="Live updates" checked={on} onChange={setOn} disabled={disabled}>
      Live updates
    </Switch>
  );
}

test("a switch toggles and reports its state", () => {
  render(<Harness />);
  const toggle = screen.getByRole("switch", { name: "Live updates" });
  expect(toggle).not.toBeChecked();
  fireEvent.click(toggle);
  expect(toggle).toBeChecked();
  fireEvent.click(toggle);
  expect(toggle).not.toBeChecked();
});

test("a disabled switch does not toggle", () => {
  render(<Harness disabled />);
  const toggle = screen.getByRole("switch", { name: "Live updates" });
  fireEvent.click(toggle);
  expect(toggle).not.toBeChecked();
});
