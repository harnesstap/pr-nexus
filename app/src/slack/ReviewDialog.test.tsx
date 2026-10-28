import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { Api, ListRow, PrDetail, ReviewPreview, ReviewTarget, SlackIndicator, TargetChoice } from "../api";
import { ReviewDialog, withNote } from "./ReviewDialog";

const row = {
  id: 7,
  number: 42,
  title: "Add passkeys",
  url: "https://github.com/acme/widgets/pull/42",
  repo: "acme/widgets",
  slack: null,
} as ListRow;

const detail = { mandatory_teams: ["@acme/platform"], candidate_teams: [] } as unknown as PrDetail;
const mapped: TargetChoice = {
  target: { workspace: "acme", channel_id: "C0123456789", channel_name: "platform" },
  teams: ["@acme/platform"],
  source: "mapping",
};

function fake(overrides: Partial<Api> = {}) {
  const calls = { targets: [] as string[][], sent: [] as [ReviewTarget, string[]][], notes: [] as (string | undefined)[], order: [] as string[] };
  const api = {
    prDetail: () => Promise.resolve(detail),
    slackWorkspaces: () => Promise.resolve([{ id: 3, team_name: "Acme", domain: "acme", user_name: "ada", signed_in: true, needs_sign_in: false, team_id: "T0ACME0001", enabled: true, priority: 0 }]),
    reviewTargets: (_id: number, teams: string[]) => {
      calls.targets.push(teams);
      return Promise.resolve([mapped]);
    },
    reviewPreview: (_id: number, target: ReviewTarget): Promise<ReviewPreview> =>
      Promise.resolve({
        target,
        action: "request",
        text: `Review please · #${target.channel_name}`,
        warnings: [],
        pr_warnings: [],
        signed_in: true,
        bump_count: 0,
        permalink: null,
        broadcast: true,
      }),
    sendReview: (_id: number, target: ReviewTarget, teams: string[], note?: string): Promise<SlackIndicator> => {
      calls.sent.push([target, teams]);
      calls.notes.push(note);
      calls.order.push("send");
      return Promise.resolve({
        requested_at: "2026-10-26T12:00:00Z",
        last_bumped_at: null,
        bump_count: 0,
        workspace: target.workspace,
        channel_id: target.channel_id,
        channel_name: target.channel_name,
        permalink: "https://acme.slack.com/archives/C0123456789/p1",
        root_ts: "1.0",
        source: "app",
      });
    },
    setPrDraft: (_id: number, draft: boolean) => {
      calls.order.push(`draft:${draft}`);
      return Promise.resolve({});
    },
    ...overrides,
  } as unknown as Api;
  return { api, calls };
}

test("the mapped channel is previewed for the mandatory teams and Send posts it", async () => {
  const { api, calls } = fake();
  const sent: SlackIndicator[] = [];
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={(item) => sent.push(item)} />);
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("Review please · #platform");
  expect(calls.targets).toEqual([["@acme/platform"]]);
  fireEvent.click(screen.getByRole("button", { name: "Send request" }));
  await waitFor(() => expect(sent).toHaveLength(1));
  expect(calls.sent).toEqual([[mapped.target, ["@acme/platform"]]]);
});

test("a bump shows the thread link and a signed-out workspace blocks Send", async () => {
  const opened: string[] = [];
  const { api } = fake({
    reviewPreview: (_id, target) =>
      Promise.resolve({
        target,
        action: "bump",
        text: "Friendly bump",
        warnings: ["Last bump was 2 h ago"],
        pr_warnings: [],
        signed_in: false,
        bump_count: 1,
        permalink: "https://acme.slack.com/archives/C0123456789/p2",
        broadcast: true,
      }),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={(url) => opened.push(url)} onSent={() => undefined} />);
  expect(await screen.findByText(/thread, also sent to the channel \(1 earlier bump\)/)).toBeInTheDocument();
  expect(screen.getByText("Last bump was 2 h ago")).toBeInTheDocument();
  expect(screen.getByText(/Sign in to acme in Settings/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Bump in thread" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Open thread ↗" }));
  expect(opened).toEqual(["https://acme.slack.com/archives/C0123456789/p2"]);
});

test("without a mapping the channel is entered by hand and errors are shown", async () => {
  const { api } = fake({
    reviewTargets: () => Promise.resolve([]),
    reviewPreview: (_id, target) =>
      target.channel_id.length < 9
        ? Promise.reject({ kind: "validation", message: "Enter a Slack channel ID such as C0123456789" })
        : Promise.resolve({ target, action: "request", text: "hand", warnings: [], pr_warnings: [], signed_in: true, bump_count: 0, permalink: null, broadcast: true }),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  expect(await screen.findByText(/No mapped channel/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Enter an ID" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "C1" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview" }));
  expect(await screen.findByText(/such as C0123456789/)).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "C0123456789" } });
  fireEvent.click(screen.getByRole("button", { name: "Preview" }));
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("hand");
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
});

test("a send whose link is unknown is a notice, not an error, and Send stays off", async () => {
  const notice = "Sent to Slack, but the link is unknown. Check the channel before sending again.";
  const { api } = fake({ sendReview: () => Promise.reject({ kind: "sent_unconfirmed", message: notice }) });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  await screen.findByLabelText("Message preview");
  fireEvent.click(screen.getByRole("button", { name: "Send request" }));
  expect(await screen.findByRole("status")).toHaveTextContent(notice);
  expect(screen.getByRole("button", { name: "Send request" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
});

test("a searched member replaces the mapped channel and the request keeps the PR's teams", async () => {
  const { api, calls } = fake({
    searchSlackTargets: (query: string) =>
      Promise.resolve(
        query === "@grace" ? [{ kind: "member" as const, target: { workspace: "acme", channel_id: "U0ACME0002", channel_name: "Grace Hopper" } }] : [],
      ),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  await screen.findByLabelText("Message preview");
  const box = screen.getByRole("combobox", { name: "Channel or person" });
  fireEvent.focus(box);
  fireEvent.change(box, { target: { value: "@grace" } });
  fireEvent.click(await screen.findByRole("option", { name: /@Grace Hopper/ }));
  expect(await screen.findByText("New message in @Grace Hopper.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Send request" }));
  await waitFor(() => expect(calls.sent).toHaveLength(1));
  expect(calls.sent[0]).toEqual([{ workspace: "acme", channel_id: "U0ACME0002", channel_name: "Grace Hopper" }, ["@acme/platform"]]);
});

const draftRow = { ...row, state: "open", is_draft: true } as ListRow;

test("a draft PR warns at the top and can still be sent", async () => {
  const { api, calls } = fake();
  const sent: SlackIndicator[] = [];
  render(<ReviewDialog api={api} row={draftRow} onClose={() => undefined} onOpen={() => undefined} onSent={(item) => sent.push(item)} />);
  await screen.findByLabelText("Message preview");
  expect(screen.getByRole("note", { name: "Draft warning" })).toHaveTextContent("This PR is still a draft. Reviewers may skip it.");
  fireEvent.click(screen.getByRole("button", { name: "Send anyway" }));
  await waitFor(() => expect(sent).toHaveLength(1));
  expect(calls.order).toEqual(["send"]);
});

test("a ready PR shows no draft warning", async () => {
  const { api } = fake();
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  await screen.findByLabelText("Message preview");
  expect(screen.queryByRole("note", { name: "Draft warning" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
});

test("the backend's draft warning shows even when the row looked ready", async () => {
  const { api } = fake({
    reviewPreview: (_id: number, target: ReviewTarget): Promise<ReviewPreview> =>
      Promise.resolve({
        target,
        action: "request",
        text: "Hello\n",
        warnings: [],
        pr_warnings: [{ kind: "draft", pr_id: 7, number: 42, message: "#42 is still a draft." }],
        signed_in: true,
        bump_count: 0,
        permalink: null,
        broadcast: true,
      }),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  expect(await screen.findByRole("note", { name: "Draft warning" })).toBeInTheDocument();
});

test("Mark ready for review clears the warning without sending", async () => {
  const { api, calls } = fake();
  const ready: number[] = [];
  render(<ReviewDialog api={api} row={draftRow} onClose={() => undefined} onOpen={() => undefined} onReady={(id) => ready.push(id)} onSent={() => undefined} />);
  await screen.findByLabelText("Message preview");
  fireEvent.click(screen.getByRole("button", { name: "Mark ready for review" }));
  await waitFor(() => expect(screen.queryByRole("note", { name: "Draft warning" })).not.toBeInTheDocument());
  expect(ready).toEqual([7]);
  expect(calls.order).toEqual(["draft:false"]);
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
});

test("if marking ready fails, the warning stays and nothing is sent", async () => {
  const { api, calls } = fake({ setPrDraft: () => Promise.reject(new Error("no access")) });
  render(<ReviewDialog api={api} row={draftRow} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  await screen.findByLabelText("Message preview");
  fireEvent.click(screen.getByRole("button", { name: "Mark ready for review" }));
  expect(await screen.findByText(/Couldn't mark it ready for review: no access/)).toBeInTheDocument();
  expect(screen.getByRole("note", { name: "Draft warning" })).toBeInTheDocument();
  expect(calls.sent).toEqual([]);
});

test("the preview renders like Slack and shows the optional note, which is sent", async () => {
  const opened: string[] = [];
  const { api, calls } = fake({
    reviewPreview: (_id, target) =>
      Promise.resolve({
        target,
        action: "request",
        text: "Hello team\nCould someone please review *Add passkeys*? :pray:\n<https://github.com/acme/widgets/pull/42|acme/widgets#42>\n",
        warnings: [],
        pr_warnings: [],
        signed_in: true,
        bump_count: 0,
        permalink: null,
        broadcast: true,
      }),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={(url) => opened.push(url)} onSent={() => undefined} />);
  const preview = await screen.findByLabelText("Message preview");
  expect(preview.querySelector("strong")?.textContent).toBe("Add passkeys");
  expect(preview.textContent).toContain("🙏");
  expect(preview.textContent).not.toContain(":pray:");
  fireEvent.click(screen.getByRole("link", { name: "acme/widgets#42" }));
  expect(opened).toEqual(["https://github.com/acme/widgets/pull/42"]);

  const note = screen.getByRole("textbox", { name: "Note" });
  expect(note).toHaveAttribute("spellcheck", "false");
  expect(note).toHaveAttribute("autocorrect", "off");
  expect(note).toHaveAttribute("autocapitalize", "off");
  fireEvent.change(note, { target: { value: "  Start with the _login_ flow  " } });
  expect(preview.lastElementChild?.textContent).toBe("Start with the login flow");
  expect(preview.querySelector("em")?.textContent).toBe("login");
  fireEvent.click(screen.getByRole("button", { name: "Send request" }));
  await waitFor(() => expect(calls.notes).toEqual(["  Start with the _login_ flow  "]));
});

test("withNote mirrors the backend: blank line, trimmed, blank ignored", () => {
  expect(withNote("Hello\n", "  hi  ")).toBe("Hello\n\nhi\n");
  expect(withNote("Hello\n", "   ")).toBe("Hello\n");
});

test("a title with underscores previews as inline code with no Markdown warning", async () => {
  const text = "Hello team\nCould someone please review `feat: return location_ip_hash in geolocate response`?\nhttps://github.com/acme/widgets/pull/42\nThanks!\n";
  const { api } = fake({
    reviewPreview: (_id: number, target: ReviewTarget): Promise<ReviewPreview> =>
      Promise.resolve({ target, action: "request", text, warnings: [], pr_warnings: [], signed_in: true, bump_count: 0, permalink: null, broadcast: true }),
  });
  render(<ReviewDialog api={api} row={{ ...row, title: "feat: return location_ip_hash in geolocate response" }} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  const preview = await screen.findByLabelText("Message preview");
  expect(preview).toHaveTextContent("review feat: return location_ip_hash in geolocate response?");
  expect(preview.querySelector("code")).toHaveTextContent(/^feat: return location_ip_hash in geolocate response$/);
  expect(screen.queryByText(/Markdown characters/)).not.toBeInTheDocument();
});
