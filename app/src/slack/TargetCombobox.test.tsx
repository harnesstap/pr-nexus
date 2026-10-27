import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { ReviewTarget, SlackWorkspace, TargetChoice, TargetMatch } from "../api";
import { parseQuery, SEARCH_DEBOUNCE_MS, TargetCombobox } from "./TargetCombobox";

const acme: SlackWorkspace = { id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false };
const labs: SlackWorkspace = { id: 2, team_name: "Acme Labs", domain: "acme-labs", user_name: "ada", signed_in: true, needs_sign_in: false };
const mapped: TargetChoice = {
  target: { workspace: "acme", channel_id: "C0ACME0003", channel_name: "platform" },
  teams: ["@acme/platform"],
  source: "mapping",
};
const directory: TargetMatch[] = [
  { kind: "channel", target: { workspace: "acme", channel_id: "C0ACME0003", channel_name: "platform" } },
  { kind: "channel", target: { workspace: "acme", channel_id: "C0ACME0004", channel_name: "platform-alerts" } },
  { kind: "member", target: { workspace: "acme", channel_id: "U0ACME0002", channel_name: "Grace Hopper" } },
];

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function setup({ workspaces = [acme], suggestions = [mapped], fail = false } = {}) {
  const calls: [string, string][] = [];
  const picked: [ReviewTarget, TargetChoice | null][] = [];
  const api = {
    searchSlackTargets: (workspace: string, query: string) => {
      calls.push([workspace, query]);
      if (fail) {
        return Promise.reject({ kind: "slack", message: "missing_scope" });
      }
      const { channels, members, term } = parseQuery(query);
      return Promise.resolve(
        directory
          .filter((match) => (match.kind === "channel" ? channels : members))
          .filter((match) => match.target.channel_name.toLowerCase().includes(term.toLowerCase()))
          .map((match) => ({ ...match, target: { ...match.target, workspace } })),
      );
    },
  };
  function Harness() {
    const [value, setValue] = useState<ReviewTarget | null>(null);
    return (
      <TargetCombobox
        api={api}
        workspaces={workspaces}
        suggestions={suggestions}
        value={value}
        onSelect={(target, choice) => {
          picked.push([target, choice]);
          setValue(target);
        }}
      />
    );
  }
  render(<Harness />);
  const box = screen.getByRole("combobox", { name: "Channel or person" });
  return { api, calls, picked, box };
}

async function type(box: HTMLElement, text: string) {
  fireEvent.focus(box);
  fireEvent.change(box, { target: { value: text } });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS);
  });
}

test("the prefix picks what is searched", () => {
  expect(parseQuery("#plat")).toEqual({ channels: true, members: false, term: "plat" });
  expect(parseQuery(" @grace ")).toEqual({ channels: false, members: true, term: "grace" });
  expect(parseQuery("plat")).toEqual({ channels: true, members: true, term: "plat" });
});

test("search waits 250 ms after the last keystroke and asks once", async () => {
  const { box, calls } = setup();
  fireEvent.focus(box);
  fireEvent.change(box, { target: { value: "#p" } });
  fireEvent.change(box, { target: { value: "#pl" } });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(SEARCH_DEBOUNCE_MS - 1);
  });
  expect(calls).toEqual([]);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  expect(calls).toEqual([["acme", "#pl"]]);
});

test("code-owner channels come first and are not repeated by search", async () => {
  const { box } = setup();
  await type(box, "#plat");
  const options = screen.getAllByRole("option").map((item) => item.textContent);
  expect(options).toEqual(["#platform@acme/platform", "#platform-alerts"]);
  expect(screen.getByText("Code owners and open requests")).toBeInTheDocument();
  expect(screen.getByText("Channels")).toBeInTheDocument();
});

test("@ lists people, who get a direct message, and the keyboard picks one", async () => {
  const { box, picked } = setup({ suggestions: [] });
  await type(box, "@gra");
  expect(screen.getByText("People · direct message")).toBeInTheDocument();
  expect(box).toHaveAttribute("aria-expanded", "true");
  fireEvent.keyDown(box, { key: "ArrowDown" });
  const option = screen.getByRole("option", { name: /@Grace Hopper/ });
  expect(option).toHaveAttribute("aria-selected", "true");
  expect(box).toHaveAttribute("aria-activedescendant", option.id);
  fireEvent.keyDown(box, { key: "Enter" });
  expect(picked).toEqual([[{ workspace: "acme", channel_id: "U0ACME0002", channel_name: "Grace Hopper" }, null]]);
  expect(box).toHaveValue("@Grace Hopper");
  expect(box).toHaveAttribute("aria-expanded", "false");
});

test("picking a suggestion hands back the choice so its teams are recorded", async () => {
  const { box, picked } = setup();
  fireEvent.focus(box);
  fireEvent.click(screen.getByRole("option", { name: /#platform/ }));
  expect(picked).toEqual([[mapped.target, mapped]]);
});

test("the search is scoped to the chosen workspace", async () => {
  const { box, calls } = setup({ workspaces: [acme, labs], suggestions: [] });
  fireEvent.change(screen.getByRole("combobox", { name: "Workspace" }), { target: { value: "acme-labs" } });
  await type(box, "#plat");
  expect(calls).toEqual([["acme-labs", "#plat"]]);
  expect(within(screen.getByRole("listbox")).getAllByRole("option")).toHaveLength(2);
});

test("a failed search says why and an ID can still be used", async () => {
  const { box, picked } = setup({ suggestions: [], fail: true });
  await type(box, "c0acme0009");
  expect(screen.getByText("Search failed: missing_scope")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("option", { name: "Use C0ACME0009" }));
  expect(picked[0][0]).toEqual({ workspace: "acme", channel_id: "C0ACME0009", channel_name: "" });

  fireEvent.click(screen.getByRole("button", { name: "Enter an ID" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "u0acme0004" } });
  fireEvent.change(screen.getByRole("textbox", { name: "Name" }), { target: { value: "Margaret" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview" }));
  expect(picked[1][0]).toEqual({ workspace: "acme", channel_id: "U0ACME0004", channel_name: "Margaret" });
});

test("Escape closes the list without reaching the dialog", async () => {
  const { box } = setup();
  const onDocument = vi.fn();
  document.addEventListener("keydown", onDocument);
  fireEvent.focus(box);
  expect(screen.getByRole("listbox")).toBeInTheDocument();
  fireEvent.keyDown(box, { key: "Escape" });
  expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  expect(onDocument).not.toHaveBeenCalled();
  document.removeEventListener("keydown", onDocument);
});

test("the helper line explains the prefixes", () => {
  const { box } = setup();
  const help = document.getElementById(box.getAttribute("aria-describedby")!);
  expect(help).toHaveTextContent("Type # to search channels or @ to search people; a person gets a direct message.");
});
