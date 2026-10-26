import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, Bootstrap } from "../api";
import { SlackSettings } from "./SlackSettings";

const boot = {
  slack_client_id: "111.222",
  slack_redirect_uri: "",
  slack_secret_saved: true,
  slack_mentions: "off",
  bump_broadcast: true,
  slack_workspaces: [{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: false, needs_sign_in: false }],
} as unknown as Bootstrap;

test("settings show signed-out workspaces, the saved secret, and save a custom redirect", async () => {
  const saved: string[][] = [];
  const api = {
    saveSlack: (id: string, secret: string, redirect: string) => {
      saved.push([id, secret, redirect]);
      return Promise.resolve();
    },
  } as unknown as Api;
  render(<SlackSettings api={api} boot={boot} />);
  expect(screen.getByText(/signed out/)).toBeInTheDocument();
  expect(screen.getByPlaceholderText(/saved in the keychain/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Add another workspace" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Reconnect Acme" })).toBeEnabled();
  const redirect = screen.getByRole("textbox", { name: /Redirect URI/ });
  expect(redirect).toHaveValue("http://127.0.0.1:47821/mcp/oauth/callback");
  fireEvent.change(redirect, { target: { value: "http://localhost:40002/cb" } });
  fireEvent.click(screen.getByRole("button", { name: "Save Slack settings" }));
  await waitFor(() => expect(saved).toEqual([["111.222", "", "http://localhost:40002/cb"]]));
  expect(await screen.findByText("Slack settings saved.")).toBeInTheDocument();
});

test("a workspace whose token is gone shows needs sign-in and reconnects in place", async () => {
  const stale = { id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: false, needs_sign_in: true };
  const api = {
    saveSlack: () => Promise.resolve(),
    slackConnect: () => Promise.resolve({ ...stale, signed_in: true, needs_sign_in: false }),
  } as unknown as Api;
  render(<SlackSettings api={api} boot={{ ...boot, slack_workspaces: [stale] } as unknown as Bootstrap} />);
  expect(screen.getByText(/needs sign-in/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Reconnect Acme" }));
  expect(await screen.findByText("Reconnected to Acme.")).toBeInTheDocument();
  expect(screen.queryByText(/needs sign-in/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Reconnect Acme" })).not.toBeInTheDocument();
});

test("the mentions scope saves on change, warns about private results, and reports up", async () => {
  const saved: string[] = [];
  const changed: string[] = [];
  const api = {
    saveSlackMentions: (scope: string) => {
      saved.push(scope);
      return Promise.resolve();
    },
  } as unknown as Api;
  render(<SlackSettings api={api} boot={boot} onMentionsChange={(scope) => changed.push(scope)} />);
  const select = screen.getByRole("combobox", { name: /Find mentions/ });
  expect(select).toHaveValue("off");
  fireEvent.change(select, { target: { value: "all" } });
  expect(screen.getByText(/can include private channels and DMs/)).toBeInTheDocument();
  expect(await screen.findByText("Saved.")).toBeInTheDocument();
  expect(saved).toEqual(["all"]);
  expect(changed).toEqual(["all"]);
});

test("a failed mentions save reverts the select", async () => {
  const api = { saveSlackMentions: () => Promise.reject(new Error("disk full")) } as unknown as Api;
  render(<SlackSettings api={api} boot={boot} />);
  const select = screen.getByRole("combobox", { name: /Find mentions/ });
  fireEvent.change(select, { target: { value: "public" } });
  expect(await screen.findByText(/disk full/)).toBeInTheDocument();
  expect(select).toHaveValue("off");
});

test("the bump broadcast toggle defaults on, saves off, and rolls back on failure", async () => {
  const saved: boolean[] = [];
  const changed: boolean[] = [];
  const api = {
    saveBumpBroadcast: (on: boolean) => {
      saved.push(on);
      return saved.length > 1 ? Promise.reject(new Error("disk full")) : Promise.resolve();
    },
  } as unknown as Api;
  render(<SlackSettings api={api} boot={boot} onBroadcastChange={(on) => changed.push(on)} />);
  const toggle = screen.getByRole("checkbox", { name: /Also post bumps to the channel/ });
  expect(toggle).toBeChecked();
  fireEvent.click(toggle);
  await waitFor(() => expect(changed).toEqual([false]));
  expect(toggle).not.toBeChecked();
  fireEvent.click(toggle);
  expect(await screen.findByText("disk full")).toBeInTheDocument();
  expect(toggle).not.toBeChecked();
  expect(saved).toEqual([false, true]);
});
