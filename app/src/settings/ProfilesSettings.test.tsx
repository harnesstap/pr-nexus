import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, Bootstrap, ProfileDto } from "../api";
import { ProfilesSettings, deleteBlocker } from "./ProfilesSettings";

const work: ProfileDto = { id: "default", name: "Work", active: false, githubLogin: "ada", isDefault: true };
const labs: ProfileDto = { id: "acme-labs", name: "Acme Labs", active: false, githubLogin: null, isDefault: false };

test("the default, the active and the last profile can't be deleted", () => {
  expect(deleteBlocker(labs, 1)).toMatch(/last/);
  expect(deleteBlocker(work, 2)).toMatch(/default/);
  expect(deleteBlocker({ ...labs, active: true }, 2)).toMatch(/Switch/);
  expect(deleteBlocker(labs, 2)).toBeNull();
});

test("renames and deletes after confirming", async () => {
  let list = [{ ...work, active: true }, labs];
  const calls: string[] = [];
  const api = {
    listProfiles: () => Promise.resolve(list),
    renameProfile: (id: string, name: string) => {
      calls.push(`rename ${id} ${name}`);
      list = list.map((p) => (p.id === id ? { ...p, name } : p));
      return Promise.resolve(list);
    },
    deleteProfile: (id: string) => {
      calls.push(`delete ${id}`);
      list = list.filter((p) => p.id !== id);
      return Promise.resolve(list);
    },
  } as unknown as Api;
  render(<ProfilesSettings api={api} boot={{ profiles: list } as Bootstrap} onBoot={() => undefined} />);

  fireEvent.click(screen.getAllByRole("button", { name: "Rename" })[1]);
  fireEvent.change(screen.getByLabelText("Profile name"), { target: { value: "Acme Side" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Acme Side")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(calls).toEqual(["rename acme-labs Acme Side"]);
  const confirm = screen.getByRole("alertdialog", { name: "Delete Acme Side" });
  expect(confirm).toHaveTextContent(/keychain/);
  fireEvent.click(screen.getByRole("button", { name: "Delete profile" }));
  await waitFor(() => expect(screen.queryByText("Acme Side")).not.toBeInTheDocument());
  expect(calls).toEqual(["rename acme-labs Acme Side", "delete acme-labs"]);
});
