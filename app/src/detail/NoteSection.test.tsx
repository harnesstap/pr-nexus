import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api } from "../api";
import { NoteSection } from "./NoteSection";

function fakeApi(initial = "") {
  return {
    prNote: vi.fn((prId: number) => Promise.resolve({ pr_id: prId, note: initial, updated_at: null })),
    setPrNote: vi.fn((prId: number, note: string) => Promise.resolve({ pr_id: prId, note: note.trim(), updated_at: "2026-10-26T10:00:00Z" })),
  } as unknown as Api & { setPrNote: ReturnType<typeof vi.fn> };
}

test("the note loads, saves on blur and not again when unchanged", async () => {
  const api = fakeApi("Check the migration");
  render(<NoteSection api={api} prId={7} />);
  const field = await screen.findByDisplayValue("Check the migration");
  fireEvent.blur(field);
  expect(api.setPrNote).not.toHaveBeenCalled();
  fireEvent.change(field, { target: { value: "Ask Lee about rollout " } });
  expect(screen.getByRole("status")).toHaveTextContent("Unsaved");
  fireEvent.blur(field);
  await waitFor(() => expect(api.setPrNote).toHaveBeenCalledWith(7, "Ask Lee about rollout "));
  expect(await screen.findByDisplayValue("Ask Lee about rollout")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent(/Saved/);
});

test("Save writes the note explicitly and errors show under the field", async () => {
  const api = fakeApi();
  api.setPrNote.mockImplementationOnce(() => Promise.reject(new Error("Note is too long")));
  render(<NoteSection api={api} prId={3} />);
  const field = await screen.findByRole("textbox", { name: "Note for this pull request" });
  const save = screen.getByRole("button", { name: "Save note" });
  expect(save).toBeDisabled();
  fireEvent.change(field, { target: { value: "x" } });
  fireEvent.click(save);
  expect(await screen.findByText("Note is too long")).toBeInTheDocument();
  fireEvent.click(save);
  await waitFor(() => expect(api.setPrNote).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(screen.queryByText("Note is too long")).toBeNull());
});
