import { useEffect, useState, type ReactNode } from "react";
import type { Api, McpCapability, McpSettings, McpToolInfo, McpView, SlackSendMode, SlackWorkspace } from "../api";
import { errorMessage } from "../api";
import { ChevronRightIcon } from "../icons";
import { Switch } from "../Switch";
import { Tooltip } from "../Tooltip";
import { categoryEnabled, toolOn, toolSummary, withTool, withTools } from "./agentTools";
import { McpActivity } from "./McpActivity";
import { SlackProposals } from "./SlackProposals";
import { plainText } from "../textInput";
import { McpClientConfig } from "./McpClientConfig";
import { parseExtraHost, parsePort } from "./tabs";

export type AgentsSection = "server" | "permissions" | "client" | "proposals" | "activity";

interface Props {
  api: Pick<Api, "mcpSettings" | "mcpTools" | "saveMcpSettings" | "mcpToken" | "regenerateMcpToken" | "mcpClientStates" | "mcpAddToClient" | "mcpOpenClientConfig" | "mcpActivity" | "clearMcpActivity" | "reviewStats" | "openExternal" | "slackWorkspaces" | "slackConnect" | "mcpSlackProposals" | "decideMcpSlackProposal">;
  /** Which block to show. Omit to show every block. */
  section?: AgentsSection;
  /** Opens another Agents section when only one section is on screen. */
  onOpenSection?: (section: AgentsSection) => void;
}

const button = "rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800 disabled:opacity-50";

/** Settings → Agents: server, permissions, client, proposals and activity. */
export function AgentsSettings({ api, section, onOpenSection }: Props) {
  const [view, setView] = useState<McpView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [port, setPort] = useState("");
  const [portError, setPortError] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>([]);
  const [reauthing, setReauthing] = useState<string | null>(null);
  // Workspaces signed in again that still lack scopes: the Slack app itself
  // does not list them, so another sign-in alone cannot help.
  const [notGranted, setNotGranted] = useState<string[]>([]);
  const [tools, setTools] = useState<McpToolInfo[]>([]);
  const [toolsError, setToolsError] = useState<string | null>(null);
  const [openTools, setOpenTools] = useState<ReadonlySet<McpCapability>>(() => new Set());

  useEffect(() => {
    let cancel = false;
    api
      .mcpSettings()
      .then((next) => {
        if (!cancel) {
          setView(next);
          setPort(next.settings.port?.toString() ?? "");
        }
      })
      .catch((err: unknown) => !cancel && setError(errorMessage(err)));
    api
      .slackWorkspaces()
      .then((list) => !cancel && setWorkspaces(list))
      .catch(() => undefined);
    return () => {
      cancel = true;
    };
  }, [api]);

  useEffect(() => {
    let cancel = false;
    api
      .mcpTools()
      .then((list) => {
        if (!cancel) {
          setTools(list);
        }
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setToolsError(errorMessage(err));
        }
      });
    return () => {
      cancel = true;
    };
  }, [api]);

  function run(work: () => Promise<McpView>) {
    setBusy(true);
    setError(null);
    work()
      .then((next) => setView(next))
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false));
  }

  function save(patch: Partial<McpSettings>) {
    if (view) {
      run(() => api.saveMcpSettings({ ...view.settings, ...patch }));
    }
  }

  function toggleTools(category: McpCapability) {
    setOpenTools((current) => {
      const next = new Set(current);
      if (next.has(category)) {
        next.delete(category);
      } else {
        next.add(category);
      }
      return next;
    });
  }

  function applyPort() {
    const parsed = parsePort(port);
    if ("error" in parsed) {
      setPortError(parsed.error);
      return;
    }
    setPortError(null);
    if (parsed.port !== view?.settings.port) {
      save({ port: parsed.port });
    }
  }

  function reauth(teamId: string) {
    setBusy(true);
    setError(null);
    setReauthing(teamId);
    api
      .slackConnect()
      .then(() => api.slackWorkspaces())
      .then((list) => {
        setWorkspaces(list);
        const stillMissing = list.some((ws) => ws.team_id === teamId && (ws.scopes_missing ?? []).length > 0);
        setNotGranted((prev) => (stillMissing ? [...prev.filter((id) => id !== teamId), teamId] : prev.filter((id) => id !== teamId)));
      })
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => {
        setBusy(false);
        setReauthing(null);
      });
  }

  function copy(label: string, text: Promise<string> | string) {
    void Promise.resolve(text)
      .then((value) => navigator.clipboard?.writeText(value))
      .then(() => setCopied(label))
      .catch((err: unknown) => setError(errorMessage(err)));
  }

  if (!view) {
    return error ? (
      <p role="alert" className="text-sm text-rose-300">
        Could not read the agent settings: {error}
      </p>
    ) : (
      <p className="text-sm text-slate-400">Loading…</p>
    );
  }

  const { settings, status } = view;
  const url = status.url;
  const show = (part: AgentsSection) => section === undefined || section === part;
  return (
    <div className="space-y-8 text-sm">
      {show("server") && (
      <section aria-labelledby="agents-server">
        <h2 id="agents-server" className="text-base font-semibold">
          Local MCP server
        </h2>
        <p className="mt-2 max-w-prose text-slate-300">
          Lets AI agents on this computer (Claude Code, Cursor, any MCP client) read and, if you allow it, organize your projects and pull requests. It listens
          on 127.0.0.1 only and every request needs the token below.
        </p>
        <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2">
          <Switch label="Enable MCP server" checked={settings.enabled} disabled={busy} onChange={(enabled) => save({ enabled })}>
            Enable MCP server
          </Switch>
          <span role="status" className="text-xs">
            {status.state === "running" && <span className="text-emerald-300">● Running on port {status.port}</span>}
            {status.state === "off" && <span className="text-slate-400">○ Off</span>}
            {status.state === "error" && <span className="text-rose-300">Not running: {status.error}</span>}
          </span>
        </div>
        <div className="mt-3 flex flex-wrap items-end gap-2">
          <label className="block">
            <span className="text-slate-400">Port</span>
            <input
              {...plainText}
              className="mt-1 block w-36 rounded border border-slate-700 bg-slate-900 px-2 py-1"
              inputMode="numeric"
              placeholder="Any free port"
              value={port}
              aria-invalid={portError ? true : undefined}
              aria-describedby={portError ? "agents-port-error" : undefined}
              onChange={(event) => setPort(event.target.value)}
              onKeyDown={(event) => event.key === "Enter" && applyPort()}
            />
          </label>
          <button type="button" className={button} disabled={busy} onClick={applyPort}>
            Apply port
          </button>
        </div>
        {portError && (
          <p id="agents-port-error" role="alert" className="mt-1 text-xs text-rose-300">
            {portError}
          </p>
        )}
        <ExtraAllowedHosts
          hosts={settings.extra_allowed_hosts}
          disabled={busy}
          onChange={(extra_allowed_hosts) => save({ extra_allowed_hosts })}
        />
        <dl className="mt-4 space-y-2">
          <div>
            <dt className="text-slate-400">Endpoint</dt>
            <dd className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 truncate font-mono text-xs">{url ?? "Starts when the server is on"}</span>
              {url && (
                <button
                  type="button"
                  aria-label="Copy endpoint"
                  className="shrink-0 rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                  onClick={() => copy("endpoint", url)}
                >
                  Copy
                </button>
              )}
            </dd>
          </div>
          <div>
            <dt className="text-slate-400">Token</dt>
            <dd className="flex min-w-0 items-center gap-2">
              <span className="min-w-0 truncate font-mono text-xs" aria-label="Token, masked">
                {view.token_masked ?? "Created when the server first starts"}
              </span>
              {view.token_masked && (
                <button
                  type="button"
                  aria-label="Copy token"
                  className="shrink-0 rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                  onClick={() => copy("token", api.mcpToken())}
                >
                  Copy
                </button>
              )}
            </dd>
          </div>
        </dl>
        <div className="mt-4">
          {confirming ? (
            <div
              role="group"
              aria-label="Confirm new token"
              className="flex flex-wrap items-center gap-2 rounded border border-amber-700/60 bg-amber-950/30 p-2"
            >
              <span className="text-amber-100">Agents using the current token stop working until you give them the new one.</span>
              <button
                type="button"
                className={`${button} border-amber-600`}
                disabled={busy}
                onClick={() => {
                  setConfirming(false);
                  run(() => api.regenerateMcpToken());
                }}
              >
                Regenerate token
              </button>
              <button type="button" className={button} onClick={() => setConfirming(false)}>
                Cancel
              </button>
            </div>
          ) : (
            <button type="button" className={button} disabled={busy || !view.token_masked} onClick={() => setConfirming(true)}>
              Regenerate token…
            </button>
          )}
        </div>
      </section>
      )}

      {show("permissions") && (
      <section aria-labelledby="agents-capabilities">
        <h2 id="agents-capabilities" className="text-base font-semibold">
          What agents may do
        </h2>
        {toolsError && (
          <p role="alert" className="mt-2 text-xs text-rose-300">
            Could not load the tool list: {toolsError}
          </p>
        )}
        <ul className="mt-3 space-y-2">
          <CapabilityRow
            label="Reads"
            category="reads"
            settings={settings}
            tools={tools}
            busy={busy}
            open={openTools.has("reads")}
            onToggle={() => toggleTools("reads")}
            onChange={(reads) => save({ reads })}
            onTools={(patch) => save(patch)}
          >
            Reads <span className="text-slate-400">— list and read projects, pull requests, notes and stars</span>
          </CapabilityRow>
          <CapabilityRow
            label="Local writes"
            category="local_writes"
            settings={settings}
            tools={tools}
            busy={busy}
            open={openTools.has("local_writes")}
            onToggle={() => toggleTools("local_writes")}
            describedBy={settings.local_writes ? "agents-writes-note" : undefined}
            onChange={(local_writes) => save({ local_writes })}
            onTools={(patch) => save(patch)}
            note={
              settings.local_writes ? (
                <p id="agents-writes-note" className="mt-1 text-xs text-slate-400">
                  Agents cannot delete projects. Every call shows under{" "}
                  <SectionJump label="Activity" target="activity" section={section} onOpen={onOpenSection} inlineFallback="Activity below" />.
                </p>
              ) : null
            }
          >
            Local writes <span className="text-slate-400">— create, edit and archive projects, link PRs and JIRA issues, set notes and stars</span>
          </CapabilityRow>
          <CapabilityRow
            label="Slack reads"
            category="slack_reads"
            settings={settings}
            tools={tools}
            busy={busy}
            open={openTools.has("slack_reads")}
            onToggle={() => toggleTools("slack_reads")}
            describedBy="agents-slack-note"
            onChange={(slack_reads) => save({ slack_reads })}
            onTools={(patch) => save(patch)}
            note={
              <p id="agents-slack-note" className="mt-1 max-w-prose text-xs text-slate-400">
                Each workspace answers through its own Slack sign-in. Slack replies reach agents as Slack returns them, marked as
                untrusted content.
              </p>
            }
            details={
              settings.slack_reads ? (
                <SlackWorkspacePicker
                  workspaces={workspaces}
                  selected={settings.slack_workspaces}
                  disabled={busy}
                  reauthing={reauthing}
                  notGranted={notGranted}
                  onChange={(slack_workspaces) => save({ slack_workspaces })}
                  onReauth={reauth}
                />
              ) : null
            }
          >
            Slack reads <span className="text-slate-400">— search and read public channels and your channels in the workspaces ticked below</span>
          </CapabilityRow>
          <CapabilityRow
            label="Slack private search"
            category="slack_private_search"
            settings={settings}
            tools={tools}
            busy={busy}
            switchDisabled={!settings.slack_reads}
            open={openTools.has("slack_private_search")}
            onToggle={() => toggleTools("slack_private_search")}
            describedBy="agents-private-note"
            onChange={(slack_private_search) => save({ slack_private_search })}
            onTools={(patch) => save(patch)}
            note={
              <p id="agents-private-note" className="mt-1 max-w-prose text-xs text-slate-400">
                {settings.slack_reads ? "Off by default. Applies to the same ticked workspaces." : "Turn on Slack reads first."}
              </p>
            }
          >
            Slack private search <span className="text-slate-400">— also search private channels, DMs and group DMs you are in</span>
          </CapabilityRow>
          <CapabilityRow
            label="Slack sends"
            category="slack_sends"
            settings={settings}
            tools={tools}
            busy={busy}
            switchDisabled={!settings.slack_reads}
            open={openTools.has("slack_sends")}
            onToggle={() => toggleTools("slack_sends")}
            describedBy="agents-sends-note"
            onChange={(slack_sends) => save({ slack_sends })}
            onTools={(patch) => save(patch)}
            note={
              <p id="agents-sends-note" className="mt-1 max-w-prose text-xs text-slate-400">
                {settings.slack_reads ? "Each write names one workspace." : "Turn on Slack reads first."}
              </p>
            }
            details={
              settings.slack_reads && settings.slack_sends ? (
                <>
                  <SendModes mode={settings.slack_send_mode} disabled={busy} onChange={(slack_send_mode) => save({ slack_send_mode })} />
                  {settings.slack_send_mode !== "automatic" && (
                    <label className="mt-2 ml-1 flex items-start gap-2 text-sm">
                      <input
                        type="checkbox"
                        className="mt-1"
                        checked={settings.slack_elicit}
                        disabled={busy}
                        onChange={(e) => save({ slack_elicit: e.target.checked })}
                      />
                      <span>
                        Ask in the agent when it supports it
                        <span className="block text-xs text-slate-400">
                          A held write is shown in the agent to confirm; unanswered ones still wait{" "}
                          <SectionJump label="Proposals" target="proposals" section={section} onOpen={onOpenSection} inlineFallback="here" lead="under " />.
                        </span>
                      </span>
                    </label>
                  )}
                  {settings.slack_send_mode === "automatic" && (
                    <p role="note" className="mt-1 ml-1 max-w-prose text-xs text-amber-300">
                      Agents&apos; messages are sent immediately as you, with no approval.
                    </p>
                  )}
                  {settings.slack_send_mode === "allowlist" && (
                    <AutoSendChannels channels={settings.slack_auto_send_channels} disabled={busy} onChange={(slack_auto_send_channels) => save({ slack_auto_send_channels })} />
                  )}
                </>
              ) : null
            }
          >
            Slack sends <span className="text-slate-400">— agents send messages, drafts, schedules and reactions as you</span>
          </CapabilityRow>
        </ul>
      </section>
      )}

      {show("client") && (
      <section aria-labelledby="agents-connect">
        <h2 id="agents-connect" className="text-base font-semibold">
          Connect a client
        </h2>
        {url && view.token_masked ? (
          <McpClientConfig
            url={url}
            tokenMasked={view.token_masked}
            fixedPort={settings.port != null}
            api={api}
            tokenVersion={view.token_masked}
            onCopy={(label, build) => copy(label, api.mcpToken().then(build))}
            onFixPort={section !== undefined && onOpenSection ? () => onOpenSection("server") : undefined}
          />
        ) : (
          <p className="mt-2 max-w-prose text-slate-400">Turn the server on to connect a client. The token is on Server.</p>
        )}
      </section>
      )}

      {show("proposals") && settings.slack_reads && <SlackProposals api={api} canSend={settings.slack_sends} />}
      {section === "proposals" && !settings.slack_reads && (
        <p className="text-slate-400">Turn on Slack reads under Permissions to see proposals.</p>
      )}

      {show("activity") && <McpActivity api={api} />}
      {copied && (
        <p role="status" className="text-xs text-slate-300">
          Copied {copied}.
        </p>
      )}
      {error && (
        <p role="alert" className="text-rose-300">
          {error}
        </p>
      )}
    </div>
  );
}

/** A section name. On the combined page it stays the old nearby wording. */
function SectionJump({
  label,
  target,
  section,
  onOpen,
  inlineFallback,
  lead = "",
}: {
  label: string;
  target: AgentsSection;
  section?: AgentsSection;
  onOpen?: (section: AgentsSection) => void;
  inlineFallback: string;
  lead?: string;
}) {
  if (section === undefined) {
    return <>{inlineFallback}</>;
  }
  if (!onOpen) {
    return (
      <>
        {lead}
        {label}
      </>
    );
  }
  return (
    <>
      {lead}
      <button type="button" className="text-sky-300 hover:underline" onClick={() => onOpen(target)}>
        {label}
      </button>
    </>
  );
}

interface CapabilityProps {
  label: string;
  category: McpCapability;
  settings: McpSettings;
  tools: McpToolInfo[];
  busy: boolean;
  /** The category switch itself is disabled (Slack reads is off). */
  switchDisabled?: boolean;
  open: boolean;
  describedBy?: string;
  onToggle: () => void;
  onChange: (on: boolean) => void;
  onTools: (patch: Pick<McpSettings, "disabled_tools" | "enabled_slack_send_tools">) => void;
  note?: ReactNode;
  details?: ReactNode;
  children: ReactNode;
}

/** A capability switch, with its tools collapsed on the same row. */
function CapabilityRow({
  label,
  category,
  settings,
  tools,
  busy,
  switchDisabled = false,
  open,
  describedBy,
  onToggle,
  onChange,
  onTools,
  note,
  details,
  children,
}: CapabilityProps) {
  const mine = tools.filter((tool) => tool.category === category);
  const selected = mine.filter((tool) => toolOn(tool, settings)).length;
  const checked = categoryEnabled(settings, category);
  const locked = busy || !checked;
  const summary = toolSummary(selected, mine.length);
  const panelId = `agents-tools-${category}`;
  return (
    <li>
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <Switch
            label={label}
            checked={checked}
            disabled={busy || switchDisabled}
            aria-describedby={describedBy}
            onChange={onChange}
          >
            {children}
          </Switch>
        </div>
        {mine.length > 0 && (
          <button
            type="button"
            aria-expanded={open}
            aria-controls={panelId}
            aria-label={`${label}: ${summary}`}
            className="mt-0.5 inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-xs text-slate-400 hover:bg-slate-800 hover:text-slate-200"
            onClick={onToggle}
          >
            <ChevronRightIcon size={12} className={open ? "rotate-90" : ""} />
            {summary}
          </button>
        )}
      </div>
      {note}
      {open && mine.length > 0 && (
        <div id={panelId} className="mt-1 ml-11">
          <div className="mb-1 flex gap-3 text-xs">
            <button
              type="button"
              className="text-sky-300 hover:underline disabled:cursor-not-allowed disabled:opacity-40 disabled:no-underline"
              disabled={locked}
              aria-label={`Enable all ${label} tools`}
              onClick={() => onTools(withTools(settings, mine, true))}
            >
              All
            </button>
            <button
              type="button"
              className="text-sky-300 hover:underline disabled:cursor-not-allowed disabled:opacity-40 disabled:no-underline"
              disabled={locked}
              aria-label={`Enable no ${label} tools`}
              onClick={() => onTools(withTools(settings, mine, false))}
            >
              None
            </button>
          </div>
          <ul className="space-y-1">
            {mine.map((tool) => {
              const on = toolOn(tool, settings);
              return (
                <li key={tool.name} className="flex items-start gap-2">
                  <Switch
                    compact
                    label={tool.name}
                    checked={on}
                    disabled={locked}
                    onChange={(next) => onTools(withTool(settings, tool, next))}
                  >
                    <code className={`font-mono text-xs ${locked ? "text-slate-500" : "text-slate-200"}`}>{tool.name}</code>
                  </Switch>
                  <span className={`min-w-0 pt-0.5 text-xs ${locked ? "text-slate-500" : "text-slate-400"}`}>{tool.description}</span>
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {details}
    </li>
  );
}

interface PickerProps {
  workspaces: SlackWorkspace[];
  selected: string[];
  disabled: boolean;
  /** Team being signed in again, if any. */
  reauthing: string | null;
  onChange: (teamIds: string[]) => void;
  onReauth: (teamId: string) => void;
  /** Team IDs where a new sign-in still did not get every scope. */
  notGranted?: string[];
}

/** The Slack workspaces agents may see. None is ticked until the user picks. */
function SlackWorkspacePicker({ workspaces, selected, disabled, reauthing, notGranted = [], onChange, onReauth }: PickerProps) {
  if (workspaces.length === 0) {
    return <p className="ml-11 mt-2 text-xs text-slate-400">No Slack workspace connected yet. Connect one in Settings → Slack.</p>;
  }
  const toggle = (teamId: string, on: boolean) =>
    onChange(on ? [...selected.filter((id) => id !== teamId), teamId] : selected.filter((id) => id !== teamId));
  return (
    <fieldset className="ml-11 mt-2">
      <legend className="text-xs text-slate-400">Workspaces agents may read</legend>
      <ul className="mt-1 space-y-1">
        {workspaces.map((ws) => {
          const unavailable = !ws.enabled ? "turned off in Settings → Slack" : ws.needs_sign_in ? "needs signing in again" : null;
          const missing = ws.scopes_missing ?? [];
          const noteId = `agents-scopes-${ws.team_id}`;
          return (
            <li key={ws.team_id} className="flex items-start gap-x-3">
              <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2">
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    className="accent-sky-500"
                    checked={selected.includes(ws.team_id)}
                    disabled={disabled}
                    onChange={(event) => toggle(ws.team_id, event.target.checked)}
                  />
                  <span>{ws.team_name}</span>
                  <span className="text-xs text-slate-500">{ws.domain}.slack.com</span>
                  {unavailable && <span className="text-xs text-amber-300">— {unavailable}</span>}
                </label>
                {!unavailable && missing.length > 0 &&
                  (notGranted.includes(ws.team_id) ? (
                    <span id={noteId} className="min-w-0 flex-1 text-xs text-amber-300">
                      Slack did not give these permissions: <code>{missing.join(", ")}</code>. Your Slack app does not
                      list them. Open your app on api.slack.com, go to OAuth &amp; Permissions, add them under User Token
                      Scopes, then sign in again.
                    </span>
                  ) : (
                    <Tooltip label="Missing permissions" detail={missing.join(", ")} align="start" className="min-w-0 flex-1">
                      <span id={noteId} className="min-w-0 text-xs text-amber-300">
                        — needs new Slack permissions for reactions, files, canvases and lists
                      </span>
                    </Tooltip>
                  ))}
              </div>
              {!unavailable && missing.length > 0 && (
                <button
                  type="button"
                  className="shrink-0 whitespace-nowrap rounded px-2 py-0.5 text-xs text-sky-300 hover:bg-slate-800 disabled:opacity-50"
                  aria-label={`Sign in again to ${ws.team_name}`}
                  aria-describedby={noteId}
                  disabled={disabled}
                  onClick={() => onReauth(ws.team_id)}
                >
                  {reauthing === ws.team_id ? "Waiting for Slack…" : "Sign in again"}
                </button>
              )}
            </li>
          );
        })}
      </ul>
    </fieldset>
  );
}

/** Hostnames the local server accepts besides 127.0.0.1, localhost and [::1]. */
function ExtraAllowedHosts({ hosts, disabled, onChange }: { hosts: string[]; disabled: boolean; onChange: (next: string[]) => void }) {
  const [draft, setDraft] = useState("");
  const [invalid, setInvalid] = useState<string | null>(null);
  function add() {
    const parsed = parseExtraHost(draft);
    if ("error" in parsed) {
      setInvalid(parsed.error);
      return;
    }
    setInvalid(null);
    setDraft("");
    if (!hosts.some((host) => host.toLowerCase() === parsed.host.toLowerCase())) {
      onChange([...hosts, parsed.host]);
    }
  }
  return (
    <div className="mt-3 max-w-xl">
      <label htmlFor="agents-extra-host" className="text-slate-400">
        Extra allowed hosts
      </label>
      <p id="agents-extra-hosts-hint" className="mt-1 text-xs text-slate-400">
        For clients in Docker, add host.docker.internal. Docker Desktop on macOS and Windows reaches this server on 127.0.0.1 under that name.
      </p>
      <ul aria-label="Extra allowed hosts" className="mt-2 space-y-1">
        {hosts.map((host) => (
          <li key={host} className="flex items-center gap-2">
            <span className="font-mono text-xs">{host}</span>
            <button
              type="button"
              disabled={disabled}
              aria-label={`Remove ${host}`}
              className="rounded px-1.5 text-xs hover:bg-slate-800"
              onClick={() => onChange(hosts.filter((item) => item !== host))}
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
      <form
        className="mt-2 flex items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          add();
        }}
      >
        <input
          id="agents-extra-host"
          aria-invalid={invalid ? true : undefined}
          aria-describedby={invalid ? "agents-extra-host-error" : "agents-extra-hosts-hint"}
          placeholder="host.docker.internal"
          value={draft}
          disabled={disabled}
          {...plainText}
          onChange={(event) => {
            setDraft(event.target.value);
            setInvalid(null);
          }}
          className="w-72 rounded border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-xs"
        />
        <button type="submit" disabled={disabled || draft.trim() === ""} className={button}>
          Add host
        </button>
      </form>
      {invalid && (
        <p id="agents-extra-host-error" role="alert" className="mt-1 text-xs text-rose-300">
          {invalid}
        </p>
      )}
    </div>
  );
}

const CHANNEL = /^[^\s/]+\/[CDG][A-Z0-9]+$/;

/** Channels where Slack writes go out without approval. */
function AutoSendChannels({ channels, disabled, onChange }: { channels: string[]; disabled: boolean; onChange: (next: string[]) => void }) {
  const [draft, setDraft] = useState("");
  const [invalid, setInvalid] = useState(false);
  function add() {
    const id = draft.trim();
    if (!CHANNEL.test(id)) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    setDraft("");
    if (!channels.includes(id)) {
      onChange([...channels, id]);
    }
  }
  return (
    <div className="mt-2 ml-1 space-y-1">
      <p className="text-xs text-slate-400">Auto-send channels {channels.length === 0 && "— none: every write waits for you"}</p>
      <ul aria-label="Auto-send channels" className="space-y-1">
        {channels.map((id) => (
          <li key={id} className="flex items-center gap-2">
            <span className="font-mono text-xs">{id}</span>
            <button type="button" disabled={disabled} aria-label={`Remove ${id}`} className="rounded px-1.5 text-xs hover:bg-slate-800" onClick={() => onChange(channels.filter((c) => c !== id))}>
              Remove
            </button>
          </li>
        ))}
      </ul>
      <form
        className="flex items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          add();
        }}
      >
        <input
          aria-label="Add auto-send channel"
          aria-invalid={invalid}
          placeholder="workspace/C0123456789"
          value={draft}
          disabled={disabled}
          {...plainText}
          onChange={(event) => setDraft(event.target.value)}
          className="w-64 rounded border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-xs"
        />
        <button type="submit" disabled={disabled || draft.trim() === ""} className={button}>
          Add
        </button>
      </form>
      {invalid && <p role="alert" className="text-xs text-rose-300">Use a qualified channel ID, as Slack tools return it: workspace/C…</p>}
    </div>
  );
}

const SEND_MODES: { mode: SlackSendMode; label: string; hint: string }[] = [
  { mode: "approve_all", label: "Approve everything", hint: "every write waits for you in the app" },
  { mode: "allowlist", label: "Channel allowlist", hint: "listed channels go out at once, unless the agent read another workspace first; others wait" },
  { mode: "automatic", label: "Fully automatic", hint: "every write goes out at once" },
];

function SendModes({ mode, disabled, onChange }: { mode: SlackSendMode; disabled: boolean; onChange: (mode: SlackSendMode) => void }) {
  return (
    <fieldset className="mt-2 ml-1 space-y-1 text-sm">
      <legend className="text-xs text-slate-400">Approval</legend>
      {SEND_MODES.map((m) => (
        <label key={m.mode} className="flex items-baseline gap-2">
          <input type="radio" name="slack-send-mode" value={m.mode} checked={mode === m.mode} disabled={disabled} onChange={() => onChange(m.mode)} />
          <span>
            {m.label} <span className="text-xs text-slate-400">— {m.hint}</span>
          </span>
        </label>
      ))}
    </fieldset>
  );
}
