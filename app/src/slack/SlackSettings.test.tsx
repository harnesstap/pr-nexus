import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, Bootstrap } from "../api";
import { SlackSettings } from "./SlackSettings";

const boot = {
  slack_client_id: "111.222",
  slack_redirect_uri: "",
  slack_secret_saved: true,
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
