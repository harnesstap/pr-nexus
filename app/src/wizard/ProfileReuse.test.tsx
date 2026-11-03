import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Api, ProfileDto, ReuseFrom } from "../api";
import { ProfileReuse, reuseSelection } from "./ProfileReuse";

const profiles: ProfileDto[] = [
  { id: "default", name: "Work", active: true, githubLogin: "ada", isDefault: true },
  { id: "acme-labs", name: "Acme Labs", active: false, githubLogin: "ada-labs", isDefault: false },
];

test("only profiles with picks are sent", () => {
  const none = { github: false, slackTeams: [], templates: false, sendPolicy: false, projects: false };
  expect(reuseSelection({ default: none, "acme-labs": { ...none, projects: true } }, ["default", "acme-labs"])).toEqual([
    { profile: "acme-labs", items: { ...none, projects: true } },
  ]);
});

test("picks items from several profiles, one GitHub account at most, and creates the profile", async () => {
  const created: { name: string; reuse: ReuseFrom[] }[] = [];
  const api = {
    profileReuseOptions: (id: string) =>
      Promise.resolve({
        profile: id,
        githubLogin: id === "default" ? "ada" : "ada-labs",
        slackTeams: id === "default" ? [{ team: "T0ACME0001", name: "Acme" }] : [],
        templates: id === "default" ? 2 : 0,
        projects: 3,
        sendPolicy: true,
      }),
    createProfile: (name: string, reuse: ReuseFrom[]) => {
      created.push({ name, reuse });
      return Promise.resolve({ id: "side", name, active: true, githubLogin: null, isDefault: false });
    },
  } as unknown as Api;
  const onCreated = vi.fn();
  render(<ProfileReuse api={api} profiles={profiles} onCreated={onCreated} onCancel={() => undefined} />);
  const work = await screen.findByRole("group", { name: /Copy from Work/ });
  const labs = screen.getByRole("group", { name: /Copy from Acme Labs/ });

  fireEvent.click(screen.getByRole("button", { name: "Create and continue setup" }));
  expect(await screen.findByText("Give the profile a name.")).toBeInTheDocument();

  fireEvent.change(screen.getByLabelText("Profile name"), { target: { value: " Side " } });
  const githubIn = (group: HTMLElement) => group.querySelectorAll<HTMLInputElement>("input[type=checkbox]")[0];
  fireEvent.click(githubIn(work));
  fireEvent.click(githubIn(labs));
  expect(githubIn(work).checked).toBe(false);
  fireEvent.click(screen.getByRole("checkbox", { name: /Slack workspace Acme/ }));
  fireEvent.click(screen.getAllByRole("checkbox", { name: /Projects/ })[0]);
  fireEvent.click(screen.getByRole("button", { name: "Create and continue setup" }));

  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  const none = { github: false, slackTeams: [], templates: false, sendPolicy: false, projects: false };
  expect(created).toEqual([
    {
      name: "Side",
      reuse: [
        { profile: "default", items: { ...none, slackTeams: ["T0ACME0001"], projects: true } },
        { profile: "acme-labs", items: { ...none, github: true } },
      ],
    },
  ]);
});
