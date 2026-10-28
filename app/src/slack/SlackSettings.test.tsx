import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, Bootstrap } from "../api";
import { AccountsSettings } from "../settings/AccountsSettings";

const boot = {
  github_login: "ada",
  org_choices: ["acme"],
  tracked_orgs: ["acme"],
  include_personal: false,
  orgs_needing_sso: [],
  slack_client_id: "111.222",
  slack_redirect_uri: "",
  slack_secret_saved: true,
  slack_mentions: "off",
  bump_broadcast: true,
  slack_workspaces: [{ id: 1, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: false, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }],
} as unknown as Bootstrap;

function renderAccounts(api: Partial<Api>, next: Partial<Bootstrap> = {}, onMentionsChange: (scope: string) => void = () => undefined) {
  render(<AccountsSettings api={api as Api} boot={{ ...boot, ...next }} onOrgsSaved={() => Promise.resolve()} onMentionsChange={onMentionsChange} />);
}

test("settings show signed-out workspaces, the saved secret, and save a custom redirect from the save bar", async () => {
  const saved: string[][] = [];
  renderAccounts({
    saveSlack: (id: string, secret: string, redirect: string) => {
      saved.push([id, secret, redirect]);
      return Promise.resolve();
    },
  });
  expect(screen.getByText(/signed out/)).toBeInTheDocument();
  expect(screen.getByPlaceholderText(/saved in the keychain/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Add another workspace" })).toBeEnabled();
  expect(screen.getByRole("button", { name: "Reconnect Acme" })).toBeEnabled();
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
  const redirect = screen.getByRole("textbox", { name: /Redirect URI/ });
  expect(redirect).toHaveValue("http://127.0.0.1:47821/mcp/oauth/callback");
  fireEvent.change(redirect, { target: { value: "http://localhost:40002/cb" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(saved).toEqual([["111.222", "", "http://localhost:40002/cb"]]));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
});

test("the Slack app fields fold under advanced once configured, and open when not", () => {
  renderAccounts({});
  expect(screen.getByText(/Slack app \(advanced\) ·/).closest("details")).not.toHaveAttribute("open");
});

test("an unconfigured app opens the advanced fields and gates sign-in on the client ID", () => {
  renderAccounts({}, { slack_client_id: "", slack_workspaces: [] });
  expect(screen.getByText(/Slack app \(advanced\) ·/).closest("details")).toHaveAttribute("open");
  expect(screen.getByRole("button", { name: "Sign in with Slack" })).toBeDisabled();
  fireEvent.change(screen.getByRole("textbox", { name: /Client ID/ }), { target: { value: "333.444" } });
  expect(screen.getByRole("button", { name: "Sign in with Slack" })).toBeEnabled();
});

test("Add another workspace stays enabled with a saved client ID even when the field is cleared", () => {
  renderAccounts({});
  fireEvent.change(screen.getByRole("textbox", { name: /Client ID/ }), { target: { value: "" } });
  expect(screen.getByRole("button", { name: "Add another workspace" })).toBeEnabled();
});

test("a workspace whose token is gone shows needs sign-in and reconnects in place", async () => {
  const stale = { ...boot.slack_workspaces[0], signed_in: false, needs_sign_in: true };
  renderAccounts(
    {
      saveSlack: () => Promise.resolve(),
      slackConnect: () => Promise.resolve({ ...stale, signed_in: true, needs_sign_in: false }),
    } as unknown as Partial<Api>,
    { slack_workspaces: [stale] },
  );
  expect(screen.getByText(/needs sign-in/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Reconnect Acme" }));
  expect(await screen.findByText("Reconnected to Acme.")).toBeInTheDocument();
  expect(screen.queryByText(/needs sign-in/)).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Reconnect Acme" })).not.toBeInTheDocument();
});

test("the mentions scope warns about private results and saves from the bar", async () => {
  const saved: string[] = [];
  const changed: string[] = [];
  renderAccounts(
    {
      saveSlackMentions: (scope: string) => {
        saved.push(scope);
        return Promise.resolve();
      },
    } as unknown as Partial<Api>,
    {},
    (scope) => changed.push(scope),
  );
  const select = screen.getByRole("combobox", { name: /Find mentions/ });
  fireEvent.change(select, { target: { value: "all" } });
  expect(screen.getByText(/can include private channels and DMs/)).toBeInTheDocument();
  expect(saved).toEqual([]);
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText("Saved")).toBeInTheDocument();
  expect(saved).toEqual(["all"]);
  expect(changed).toEqual(["all"]);
});

test("a failed save keeps the edit, says why, and Discard puts it back", async () => {
  renderAccounts({ saveSlackMentions: () => Promise.reject(new Error("disk full")) } as unknown as Partial<Api>);
  const select = screen.getByRole("combobox", { name: /Find mentions/ });
  fireEvent.change(select, { target: { value: "public" } });
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByText(/disk full/)).toBeInTheDocument();
  expect(select).toHaveValue("public");
  fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(select).toHaveValue("off");
  expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
});

test("personal repos is a switch and orgs save from the bar", async () => {
  const orgs: [string[], boolean][] = [];
  renderAccounts({
    saveOrgs: (list: string[], personal: boolean) => {
      orgs.push([list, personal]);
      return Promise.resolve();
    },
  } as unknown as Partial<Api>);
  fireEvent.click(screen.getByRole("switch", { name: "Include my personal repos" }));
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(orgs).toEqual([[["acme"], true]]));
});
