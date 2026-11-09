import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import type { Toast } from "../list/model";
import type { Api, BulkPreview, BulkRequest, ListRow, MappingRow, PrDetail, ReviewPreview, ReviewTarget, SlackIndicator, TargetChoice } from "../api";
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
  expect(await screen.findByRole("alert")).toHaveTextContent("That isn't a Slack ID");
  fireEvent.change(screen.getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "C0123456789" } });
  expect(screen.queryByRole("alert")).toBeNull();
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

test("Mark ready for review reloads the preview and clears the warning without sending", async () => {
  let previews = 0;
  const { api, calls } = fake({
    reviewPreview: (_id: number, target: ReviewTarget): Promise<ReviewPreview> => {
      previews += 1;
      return Promise.resolve({
        target,
        action: "request",
        text: previews === 1 ? "While draft" : "After ready",
        warnings: [],
        pr_warnings: previews === 1 ? [{ kind: "draft", pr_id: 7, number: 42, message: "#42 is still a draft." }] : [],
        signed_in: true,
        bump_count: 0,
        permalink: null,
        broadcast: true,
      });
    },
  });
  const ready: number[] = [];
  render(<ReviewDialog api={api} row={draftRow} onClose={() => undefined} onOpen={() => undefined} onReady={(id) => ready.push(id)} onSent={() => undefined} />);
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("While draft");
  fireEvent.click(screen.getByRole("button", { name: "Mark ready for review" }));
  expect(await screen.findByText("After ready")).toBeInTheDocument();
  expect(screen.queryByRole("note", { name: "Draft warning" })).not.toBeInTheDocument();
  expect(ready).toEqual([7]);
  expect(calls.order).toEqual(["draft:false"]);
  expect(calls.sent).toEqual([]);
  expect(previews).toBe(2);
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

test("Copy message copies the previewed text with the note, without sending", async () => {
  const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
  Object.assign(navigator, { clipboard: { writeText } });
  const { api, calls } = fake();
  const notices: Toast[] = [];
  let closed = false;
  render(<ReviewDialog api={api} row={row} onClose={() => (closed = true)} onOpen={() => undefined} onSent={() => undefined} onNotice={(toast) => notices.push(toast)} />);
  await screen.findByLabelText("Message preview");
  fireEvent.change(screen.getByRole("textbox", { name: "Note" }), { target: { value: "Start with the login flow" } });
  fireEvent.click(screen.getByRole("button", { name: "Copy message" }));
  await waitFor(() => expect(writeText).toHaveBeenLastCalledWith("Review please · #platform\n\nStart with the login flow\n"));
  expect(notices.at(-1)).toEqual({ text: "Message copied" });
  expect(calls.sent).toHaveLength(0);
  expect(closed).toBe(false);
  expect(screen.getByRole("dialog")).toBeInTheDocument();
});

const acmeData: TargetChoice = {
  target: { workspace: "acme", channel_id: "C0ACME0001", channel_name: "acme-data" },
  teams: ["@acme/data"],
  source: "mapping",
};
const nameOnly: TargetChoice = {
  target: { workspace: "acme", channel_id: "", channel_name: "acme-data" },
  teams: ["@acme/data"],
  source: "unresolved",
};

test("opening the dialog preselects the code-owner channel and '#' lists it first, with no ID error", async () => {
  const { api } = fake({
    prDetail: () => Promise.resolve({ mandatory_teams: ["@acme/data"], candidate_teams: [] } as unknown as PrDetail),
    reviewTargets: () => Promise.resolve([acmeData]),
    searchSlackTargets: () => Promise.resolve([]),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  const box = await screen.findByRole("combobox", { name: "Channel or person" });
  await waitFor(() => expect(box).toHaveValue("#acme-data"));
  fireEvent.change(box, { target: { value: "#" } });
  expect(screen.getAllByRole("option")[0]).toHaveTextContent("#acme-data");
  expect(screen.queryByText(/Enter a Slack channel ID/)).toBeNull();
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
});

test("a mapping without a channel ID never prefills '#' or shows an ID error; picking it searches by name", async () => {
  const searched: string[] = [];
  const previewed: ReviewTarget[] = [];
  const { api } = fake({
    reviewTargets: () => Promise.resolve([nameOnly]),
    reviewPreview: (_id, target) => {
      previewed.push(target);
      return Promise.reject({ kind: "validation", message: "Enter a Slack channel ID such as C0123456789" });
    },
    searchSlackTargets: (query: string) => {
      searched.push(query);
      return Promise.resolve([{ kind: "channel" as const, target: acmeData.target }]);
    },
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  expect(await screen.findByText(/has no channel ID yet/)).toBeInTheDocument();
  const box = screen.getByRole("combobox", { name: "Channel or person" });
  expect(box).toHaveValue("");
  expect(previewed).toEqual([]);
  expect(screen.queryByText(/Enter a Slack channel ID/)).toBeNull();
  fireEvent.focus(box);
  fireEvent.click(screen.getByRole("option", { name: /acme-data/ }));
  await waitFor(() => expect(searched).toEqual(["#acme-data"]));
});

test("search with no results shows a grey hint, not an ID error", async () => {
  const { api } = fake({ reviewTargets: () => Promise.resolve([]), searchSlackTargets: () => Promise.resolve([]) });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  const box = await screen.findByRole("combobox", { name: "Channel or person" });
  fireEvent.change(box, { target: { value: "#nope" } });
  expect(await screen.findByText("No channel found. Try another name, or enter an ID.")).toBeInTheDocument();
  expect(screen.queryByText(/Enter a Slack channel ID/)).toBeNull();
});

test("two CODEOWNERS lines propose two sends and the team button changes the channel that is sent", async () => {
  const channel = (channel_id: string, channel_name: string): ReviewTarget => ({ workspace: "acme", channel_id, channel_name });
  const { api, calls } = fake({
    prDetail: () =>
      Promise.resolve({
        ...detail,
        owner_groups: [
          { id: "line-a", pattern: "/src/", teams: ["acme/team-a", "acme/team-b"], users: [], satisfied: false },
          { id: "line-c", pattern: "/billing/", teams: ["acme/team-c", "acme/team-d"], users: [], satisfied: false },
          { id: "line-e", pattern: "/ops/", teams: ["acme/team-e"], users: [], satisfied: false },
        ],
      }),
    bulkPreview: (request) => {
      const picked = request.group_choices?.find((choice) => choice.group_id === "line-a")?.team ?? "acme/team-a";
      const first = picked === "acme/team-b" ? channel("C0ACME0002", "team-b") : channel("C0ACME0001", "team-a");
      const second = channel("C0ACME0003", "team-c");
      const card = (target: ReviewTarget) => ({
        target,
        messages: [{ action: "request" as const, prs: [{ pr_id: 7, number: 42, title: "Add passkeys", repo: "acme/widgets" }], text: `Please review in #${target.channel_name}`, permalink: null }],
        warnings: [],
        signed_in: true,
        single_top_level: false,
        can_switch: false,
        fingerprint: target.channel_id,
      });
      return Promise.resolve({
        cards: [card(first), card(second)],
        skipped: [],
        pr_warnings: [{ kind: "checks" as const, pr_id: 7, number: 42, message: "#42 has failing checks." }],
        groups: [
          { pr_id: 7, number: 42, group_id: "line-a", pattern: "/src/", teams: ["acme/team-a", "acme/team-b"], selected: picked, hint: null, channel_id: first.channel_id, workspace: "acme" },
          { pr_id: 7, number: 42, group_id: "line-c", pattern: "/billing/", teams: ["acme/team-c", "acme/team-d"], selected: "acme/team-c", hint: null, channel_id: second.channel_id, workspace: "acme" },
          { pr_id: 7, number: 42, group_id: "line-e", pattern: "/ops/", teams: ["acme/team-e"], selected: null, hint: "No Slack channel is saved for @acme/team-e.", channel_id: null, workspace: null },
        ],
      });
    },
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  expect(await screen.findByRole("region", { name: "#team-a" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "#team-c" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "@acme/team-a" })).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByRole("button", { name: "@acme/team-b" })).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByRole("note", { name: "No Slack channel" })).toHaveTextContent("No Slack channel is saved for @acme/team-e.");
  expect(screen.getByRole("note", { name: "No Slack channel" })).toHaveClass("text-slate-500");
  expect(screen.getByRole("region", { name: "Choose a channel for @acme/team-e" })).toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Channel" })).toBeInTheDocument();
  expect(screen.getByRole("note", { name: "Failing checks" })).toHaveTextContent("#42 has failing checks.");
  fireEvent.click(screen.getByRole("button", { name: "@acme/team-b" }));
  expect(await screen.findByRole("region", { name: "#team-b" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Send 2 messages" }));
  await waitFor(() => expect(calls.sent.map(([target]) => target.channel_id)).toEqual(["C0ACME0002", "C0ACME0003"]));
});

test("one CODEOWNERS team still uses the channel picker", async () => {
  const { api, calls } = fake({
    prDetail: () => Promise.resolve({ ...detail, owner_groups: [{ id: "only", pattern: "*", teams: ["acme/platform"], users: [], satisfied: false }] }),
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  expect(await screen.findByLabelText("Message preview")).toHaveTextContent("Review please · #platform");
  expect(calls.targets).toEqual([["@acme/platform"]]);
  expect(screen.getByRole("combobox", { name: "Channel or person" })).toBeInTheDocument();
});

test("an unmapped line is a card you can address, and two lines on one channel share a message", async () => {
  const saved: MappingRow[] = [];
  const channel = (channel_id: string, channel_name: string): ReviewTarget => ({ workspace: "acme", channel_id, channel_name });
  const { api, calls } = fake({
    prDetail: () =>
      Promise.resolve({
        ...detail,
        owner_groups: [
          { id: "line-a", pattern: "/src/", teams: ["acme/team-a"], users: [], satisfied: false },
          { id: "line-b", pattern: "/billing/", teams: ["acme/team-b"], users: [], satisfied: false },
        ],
      }),
    searchSlackTargets: () => Promise.resolve([]),
    saveMapping: (row: MappingRow) => {
      saved.push(row);
      return Promise.resolve();
    },
    bulkPreview: (request: BulkRequest): Promise<BulkPreview> => {
      const skipped = new Set((request.skipped_groups ?? []).map((item) => item.group_id));
      const chosen = new Map((request.group_channels ?? []).map((item) => [item.group_id, item.target]));
      const groups: NonNullable<BulkPreview["groups"]> = [];
      const byChannel = new Map<string, ReviewTarget>();
      const add = (id: string, pattern: string, team: string, fallback: ReviewTarget | null) => {
        if (skipped.has(id)) {
          return;
        }
        const picked = chosen.get(id);
        const target = picked ? { workspace: picked.workspace || "acme", channel_id: picked.channel_id, channel_name: picked.channel_name || picked.channel_id } : fallback;
        if (!target) {
          groups.push({
            pr_id: 7,
            number: 42,
            group_id: id,
            pattern,
            teams: [team],
            selected: team,
            needs_channel: true,
            hint: `No Slack channel is saved for @${team}.`,
            channel_id: null,
            workspace: null,
          });
          return;
        }
        groups.push({
          pr_id: 7,
          number: 42,
          group_id: id,
          pattern,
          teams: [team],
          selected: team,
          needs_channel: false,
          hint: null,
          channel_id: target.channel_id,
          workspace: target.workspace,
        });
        byChannel.set(target.channel_id, target);
      };
      add("line-a", "/src/", "acme/team-a", channel("C0ACME0001", "team-a-pr-reviews"));
      add("line-b", "/billing/", "acme/team-b", null);
      return Promise.resolve({
        cards: [...byChannel.values()].map((target) => ({
          target,
          messages: [
            {
              action: target.channel_id === "C0ACME0001" ? ("bump" as const) : ("request" as const),
              prs: [{ pr_id: 7, number: 42, title: "Add passkeys", repo: "acme/widgets" }],
              text: `Please review in #${target.channel_name}`,
              permalink: target.channel_id === "C0ACME0001" ? "https://acme.slack.com/archives/C0ACME0001/p1" : null,
            },
          ],
          warnings: [],
          signed_in: true,
          single_top_level: false,
          can_switch: false,
          fingerprint: target.channel_id,
        })),
        skipped: [],
        pr_warnings: [],
        groups,
      });
    },
  });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  const pending = await screen.findByRole("region", { name: "Choose a channel for @acme/team-b" });
  expect(screen.getByRole("region", { name: "#team-a-pr-reviews" })).toBeInTheDocument();
  expect(screen.getByText("Bump in thread")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
  expect(within(pending).getByRole("note", { name: "No Slack channel" })).toHaveTextContent("No Slack channel is saved for @acme/team-b.");
  fireEvent.change(within(pending).getByRole("combobox", { name: "Channel" }), { target: { value: "#nope" } });
  expect(await within(pending).findByText("No channel found. Try another name, or enter an ID.")).toBeInTheDocument();
  expect(within(pending).queryByRole("alert")).toBeNull();
  fireEvent.click(within(pending).getByRole("button", { name: "Enter an ID" }));
  const idBox = within(pending).getByRole("textbox", { name: "Channel, DM or member ID" });
  fireEvent.change(idBox, { target: { value: "nope" } });
  fireEvent.blur(idBox);
  expect(within(pending).getByRole("alert")).toHaveTextContent("That isn't a Slack ID");
  fireEvent.change(idBox, { target: { value: "C0ACME0009" } });
  fireEvent.change(within(pending).getByRole("textbox", { name: "Name" }), { target: { value: "ops-reviews" } });
  fireEvent.click(within(pending).getByRole("button", { name: "Preview" }));
  expect(await screen.findByRole("region", { name: "#ops-reviews" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Send 2 messages" })).toBeEnabled();
  const save = screen.getByRole("checkbox", { name: "Save channel for @acme/team-b" });
  expect(save).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Change channel, currently #team-a-pr-reviews" }));
  const editor = screen.getByRole("combobox", { name: "Channel, currently #team-a-pr-reviews" });
  const header = editor.closest("section")!;
  fireEvent.click(within(header).getByRole("button", { name: "Enter an ID" }));
  fireEvent.change(within(header).getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "C0ACME0009" } });
  fireEvent.change(within(header).getByRole("textbox", { name: "Name" }), { target: { value: "ops-reviews" } });
  fireEvent.click(within(header).getByRole("button", { name: "Preview" }));
  expect(await screen.findByText("These lines share this message.")).toBeInTheDocument();
  expect(screen.getAllByRole("region", { name: "#ops-reviews" })).toHaveLength(1);
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
  fireEvent.click(screen.getByRole("checkbox", { name: "Save channel for @acme/team-a" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Save channel for @acme/team-b" }));
  fireEvent.click(screen.getByRole("button", { name: "Send request" }));
  await waitFor(() => expect(calls.sent).toHaveLength(1));
  expect(calls.sent[0]?.[0].channel_id).toBe("C0ACME0009");
  expect(calls.sent[0]?.[1]).toEqual(["acme/team-a", "acme/team-b"]);
  expect(saved).toEqual([]);
});

test("saving the picked channel writes that team's mapping, and Remove drops the card", async () => {
  const saved: MappingRow[] = [];
  const { api, calls } = fake({
    searchSlackTargets: () => Promise.resolve([]),
    saveMapping: (row: MappingRow) => {
      saved.push(row);
      return Promise.resolve();
    },
    prDetail: () =>
      Promise.resolve({
        ...detail,
        owner_groups: [
          { id: "line-a", pattern: "*", teams: ["acme/team-a"], users: [], satisfied: false },
          { id: "line-b", pattern: "/billing/", teams: ["acme/team-b"], users: [], satisfied: false },
        ],
      }),
    bulkPreview: (request: BulkRequest): Promise<BulkPreview> => {
      const skipped = new Set((request.skipped_groups ?? []).map((item) => item.group_id));
      const picked = request.group_channels?.find((choice) => choice.group_id === "line-b")?.target;
      const groups: NonNullable<BulkPreview["groups"]> = [];
      const cards: BulkPreview["cards"] = [];
      if (!skipped.has("line-a")) {
        groups.push({
          pr_id: 7,
          number: 42,
          group_id: "line-a",
          pattern: "*",
          teams: ["acme/team-a"],
          selected: "acme/team-a",
          needs_channel: false,
          hint: null,
          channel_id: "C0ACME0001",
          workspace: "acme",
        });
        cards.push({
          target: { workspace: "acme", channel_id: "C0ACME0001", channel_name: "team-a-pr-reviews" },
          messages: [{ action: "request", prs: [{ pr_id: 7, number: 42, title: "Add passkeys", repo: "acme/widgets" }], text: "Please review", permalink: null }],
          warnings: [],
          signed_in: true,
          single_top_level: false,
          can_switch: false,
          fingerprint: "C0ACME0001",
        });
      }
      if (!skipped.has("line-b")) {
        if (picked) {
          groups.push({
            pr_id: 7,
            number: 42,
            group_id: "line-b",
            pattern: "/billing/",
            teams: ["acme/team-b"],
            selected: "acme/team-b",
            needs_channel: false,
            hint: null,
            channel_id: picked.channel_id,
            workspace: picked.workspace || "acme",
          });
          cards.push({
            target: { workspace: picked.workspace || "acme", channel_id: picked.channel_id, channel_name: picked.channel_name || "ops-reviews" },
            messages: [{ action: "request", prs: [{ pr_id: 7, number: 42, title: "Add passkeys", repo: "acme/widgets" }], text: "Please review ops", permalink: null }],
            warnings: [],
            signed_in: true,
            single_top_level: false,
            can_switch: false,
            fingerprint: picked.channel_id,
          });
        } else {
          groups.push({
            pr_id: 7,
            number: 42,
            group_id: "line-b",
            pattern: "/billing/",
            teams: ["acme/team-b"],
            selected: "acme/team-b",
            needs_channel: true,
            hint: "No Slack channel is saved for @acme/team-b.",
            channel_id: null,
            workspace: null,
          });
        }
      }
      return Promise.resolve({ cards, skipped: [], pr_warnings: [], groups });
    },
  });
  const { unmount } = render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  const pending = await screen.findByRole("region", { name: "Choose a channel for @acme/team-b" });
  fireEvent.click(within(pending).getByRole("button", { name: "Enter an ID" }));
  fireEvent.change(within(pending).getByRole("textbox", { name: "Channel, DM or member ID" }), { target: { value: "C0ACME0009" } });
  fireEvent.change(within(pending).getByRole("textbox", { name: "Name" }), { target: { value: "ops-reviews" } });
  fireEvent.click(within(pending).getByRole("button", { name: "Preview" }));
  expect(await screen.findByRole("checkbox", { name: "Save channel for @acme/team-b" })).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Send 2 messages" }));
  await waitFor(() => expect(saved).toHaveLength(1));
  expect(saved[0]).toMatchObject({ github_org: "acme", team_slug: "team-b", slack_channel_id: "C0ACME0009", slack_channel_name: "ops-reviews", slack_workspace: "acme" });
  expect(calls.sent).toHaveLength(2);
  unmount();

  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  const again = await screen.findByRole("region", { name: "Choose a channel for @acme/team-b" });
  fireEvent.click(within(again).getByRole("button", { name: "Remove @acme/team-b" }));
  await waitFor(() => expect(screen.queryByRole("region", { name: "Choose a channel for @acme/team-b" })).not.toBeInTheDocument());
  expect(screen.getByRole("button", { name: "Send request" })).toBeEnabled();
});

test("failing owners are explained instead of an empty list", async () => {
  const { api } = fake({ prDetail: () => Promise.reject(new Error("GitHub is unreachable")), reviewTargets: () => Promise.resolve([]) });
  render(<ReviewDialog api={api} row={row} onClose={() => undefined} onOpen={() => undefined} onSent={() => undefined} />);
  expect(await screen.findByText(/Couldn't read this PR's code owners/)).toBeInTheDocument();
});
