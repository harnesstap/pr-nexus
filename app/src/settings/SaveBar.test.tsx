import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import { SAVED_MS, SaveBar, useSaveBar } from "./SaveBar";

afterEach(() => vi.useRealTimers());

function Form({ persist }: { persist: (value: string) => Promise<void> }) {
  const { draft, setDraft, bar } = useSaveBar("a", (value) => persist(value));
  return (
    <>
      <input aria-label="Value" value={draft} onChange={(event) => setDraft(event.target.value)} />
      <SaveBar {...bar} />
    </>
  );
}

test("the bar shows only while dirty, saves, then shows Saved briefly", async () => {
  vi.useFakeTimers();
  const saved: string[] = [];
  render(<Form persist={(value) => (saved.push(value), Promise.resolve())} />);
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "Value" }), { target: { value: "b" } });
  expect(screen.getByRole("region", { name: "Unsaved changes" })).toBeInTheDocument();
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));
  expect(saved).toEqual(["b"]);
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Saved");
  act(() => vi.advanceTimersByTime(SAVED_MS));
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

test("editing back to the saved value hides the bar; Discard restores", () => {
  render(<Form persist={() => Promise.resolve()} />);
  const input = screen.getByRole("textbox", { name: "Value" });
  fireEvent.change(input, { target: { value: "b" } });
  fireEvent.change(input, { target: { value: "a" } });
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
  fireEvent.change(input, { target: { value: "c" } });
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(input).toHaveValue("a");
});
