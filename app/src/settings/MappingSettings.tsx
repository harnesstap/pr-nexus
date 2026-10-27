import { useEffect, useState } from "react";
import type { Api, ExportMode, ImportMode, ImportPreview, MappingEntry, MappingRow, SlackWorkspace } from "../api";
import { errorMessage } from "../api";
import { WorkspaceSelect } from "../slack/WorkspaceSelect";
import { chosenWorkspace, csvHasWorkspaceColumn, rememberWorkspace } from "../slack/workspaces";
import { MAPPING_FIELDS, matchesMapping } from "./tabs";

interface Props {
  api: Api;
  /** The wizard's mapping step was skipped: say this is where it is done. */
  skippedInSetup: boolean;
  /** Called after any change so the list's mapping hint can update. */
  onCountChange?: (count: number) => void;
  /** The connected workspaces at boot; refreshed from the backend on open. */
  workspaces?: SlackWorkspace[];
  /** Opens Settings → Accounts when no workspace is connected. */
  onConnectSlack?: () => void;
}

const EMPTY_ROW: MappingRow = {
  github_org: "",
  team_slug: "",
  team_name: "",
  slack_workspace: "",
  slack_channel_name: "",
  slack_channel_id: "",
};

const box = "rounded border border-slate-700 bg-slate-900 px-2 py-1";

/** Settings → Mappings: table, inline edit, reset, CSV import with a dry run, export (§7.8, §8.2). */
export function MappingSettings({ api, skippedInSetup, onCountChange, workspaces: bootWorkspaces = [], onConnectSlack }: Props) {
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>(bootWorkspaces);
  const [entries, setEntries] = useState<MappingEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<{ row: MappingRow; disabled: boolean; isNew: boolean } | null>(null);
  const [rowError, setRowError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let cancel = false;
    api
      .listMappings()
      .then((next) => {
        if (!cancel) {
          setEntries(next);
        }
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setLoadError(errorMessage(err));
        }
      });
    return () => {
      cancel = true;
    };
  }, [api]);

  // Slack may have been connected in Accounts since boot.
  useEffect(() => {
    let cancel = false;
    api
      .slackWorkspaces()
      .then((next) => {
        if (!cancel) {
          setWorkspaces(next);
        }
      })
      .catch(() => undefined);
    return () => {
      cancel = true;
    };
  }, [api]);

  async function reload(message: string | null) {
    const next = await api.listMappings();
    setEntries(next);
    setNotice(message);
    onCountChange?.(next.length);
  }

  async function saveEdit() {
    if (!editing) {
      return;
    }
    setRowError(null);
    try {
      await api.saveMapping(editing.row, editing.disabled);
      const label = `${editing.row.github_org}/${editing.row.team_slug}`;
      setEditing(null);
      await reload(`Saved ${label}.`);
    } catch (err) {
      setRowError(errorMessage(err));
    }
  }

  async function reset(row: MappingRow) {
    try {
      await api.resetMapping(row.github_org, row.team_slug);
      await reload(`${row.github_org}/${row.team_slug} uses the imported channel again.`);
    } catch (err) {
      setNotice(errorMessage(err));
    }
  }

  const visible = (entries ?? []).filter((entry) => matchesMapping(entry.row, query));

  return (
    <div className="space-y-8">
      <section aria-labelledby="mapping-table">
        <div className="flex flex-wrap items-center gap-3">
          <h2 id="mapping-table" className="text-base font-semibold">
            Team channels
          </h2>
          <input
            type="search"
            aria-label="Filter mappings"
            placeholder="Filter teams or channels"
            title="Matches team, workspace and channel"
            className={`${box} w-72 text-sm`}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          <button
            type="button"
            className="rounded border border-slate-700 px-2 py-1 text-sm hover:bg-slate-800"
            onClick={() => {
              setRowError(null);
              setEditing({ row: { ...EMPTY_ROW, slack_workspace: chosenWorkspace(workspaces, "") }, disabled: false, isNew: true });
            }}
          >
            Add mapping
          </button>
        </div>
        <p className="mt-1 text-xs text-slate-500">
          A review request suggests the channel mapped to the requested GitHub team. Your edits win over imported rows;
          Reset to imported drops the edit.
        </p>
        {notice && (
          <p role="status" className="mt-2 text-sm text-slate-300">
            {notice}
          </p>
        )}
        {loadError ? (
          <p role="alert" className="mt-4 text-sm text-rose-300">
            Could not load mappings: {loadError}
          </p>
        ) : entries === null ? (
          <p className="mt-4 text-sm text-slate-400">Loading mappings…</p>
        ) : entries.length === 0 && !editing ? (
          <div className="mt-4 rounded border border-dashed border-slate-700 p-4 text-sm text-slate-400">
            <p className="font-medium text-slate-200">No team channels mapped yet.</p>
            <p className="mt-1">
              {skippedInSetup ? "You skipped this step in setup. " : ""}
              Import a CSV below, or add teams one by one. Without a mapping, the review dialog asks for a channel each time.
            </p>
          </div>
        ) : (
          <table className="mt-4 w-full text-left text-sm">
            <thead className="text-xs uppercase tracking-wide text-slate-500">
              <tr>
                <th className="py-1 font-medium">Team</th>
                <th className="py-1 font-medium">Workspace</th>
                <th className="py-1 font-medium">Channel</th>
                <th className="py-1 font-medium">Source</th>
                <th className="py-1 font-medium">
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {editing?.isNew && (
                <EditRow
                  editing={editing}
                  workspaces={workspaces}
                  onConnectSlack={onConnectSlack}
                  error={rowError}
                  onChange={setEditing}
                  onSave={() => void saveEdit()}
                  onCancel={() => setEditing(null)}
                />
              )}
              {visible.map((entry) => {
                const key = `${entry.row.github_org}/${entry.row.team_slug}`;
                const isEditing = editing && !editing.isNew && `${editing.row.github_org}/${editing.row.team_slug}` === key;
                if (isEditing) {
                  return (
                    <EditRow
                      key={key}
                      editing={editing}
                      workspaces={workspaces}
                      onConnectSlack={onConnectSlack}
                      error={rowError}
                      onChange={setEditing}
                      onSave={() => void saveEdit()}
                      onCancel={() => setEditing(null)}
                    />
                  );
                }
                return (
                  <tr key={key} className="border-t border-slate-800">
                    <td className="py-2">
                      <span className="font-medium">{key}</span>
                      {entry.row.team_name && <span className="block text-xs text-slate-400">{entry.row.team_name}</span>}
                    </td>
                    <td className="py-2 text-slate-300">{entry.row.slack_workspace || "—"}</td>
                    <td className="py-2 text-slate-300">
                      {entry.disabled ? (
                        <span className="text-slate-500">No channel (disabled)</span>
                      ) : (
                        <>
                          #{entry.row.slack_channel_name || entry.row.slack_channel_id}
                          {entry.row.slack_channel_id && entry.row.slack_channel_name && (
                            <span className="ml-1 font-mono text-xs text-slate-500">{entry.row.slack_channel_id}</span>
                          )}
                        </>
                      )}
                    </td>
                    <td className="py-2">
                      <span
                        className={`rounded px-1.5 py-0.5 text-xs ${
                          entry.source === "user" ? "bg-violet-500/20 text-violet-200" : "bg-slate-800 text-slate-300"
                        }`}
                      >
                        {entry.source === "user" ? "Edited" : "Imported"}
                      </span>
                    </td>
                    <td className="py-2 text-right whitespace-nowrap">
                      <button
                        type="button"
                        aria-label={`Edit ${key}`}
                        className="rounded px-2 py-1 text-xs hover:bg-slate-800"
                        onClick={() => {
                          setRowError(null);
                          setEditing({ row: entry.row, disabled: entry.disabled, isNew: false });
                        }}
                      >
                        Edit
                      </button>
                      {entry.source === "user" && entry.has_imported && (
                        <button
                          type="button"
                          aria-label={`Reset ${key} to imported`}
                          className="rounded px-2 py-1 text-xs hover:bg-slate-800"
                          onClick={() => void reset(entry.row)}
                        >
                          Reset to imported
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
              {visible.length === 0 && entries.length > 0 && (
                <tr>
                  <td colSpan={5} className="py-3 text-sm text-slate-400">
                    No mapping matches “{query}”.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        )}
      </section>
      <ImportPanel api={api} workspaces={workspaces} onConnectSlack={onConnectSlack} onImported={(rows) => void reload(`Imported ${rows} ${rows === 1 ? "row" : "rows"}.`)} />
      <ExportPanel api={api} />
    </div>
  );
}

function EditRow({
  editing,
  workspaces,
  onConnectSlack,
  error,
  onChange,
  onSave,
  onCancel,
}: {
  editing: { row: MappingRow; disabled: boolean; isNew: boolean };
  workspaces: SlackWorkspace[];
  onConnectSlack?: () => void;
  error: string | null;
  onChange: (next: { row: MappingRow; disabled: boolean; isNew: boolean }) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const set = (field: keyof MappingRow, value: string) => onChange({ ...editing, row: { ...editing.row, [field]: value } });
  return (
    <tr className="border-t border-slate-800 bg-slate-900/60 align-top">
      <td className="py-2 pr-2">
        {editing.isNew ? (
          <div className="flex gap-1">
            <input aria-label="GitHub org" placeholder="org" className={`${box} w-24`} value={editing.row.github_org} onChange={(e) => set("github_org", e.target.value)} />
            <input aria-label="Team slug" placeholder="team-slug" className={`${box} w-28`} value={editing.row.team_slug} onChange={(e) => set("team_slug", e.target.value)} />
          </div>
        ) : (
          <span className="font-medium">
            {editing.row.github_org}/{editing.row.team_slug}
          </span>
        )}
        <input aria-label="Team name" placeholder="Team name (optional)" className={`${box} mt-1 w-full`} value={editing.row.team_name} onChange={(e) => set("team_name", e.target.value)} />
      </td>
      <td className="py-2 pr-2">
        <WorkspaceSelect
          label="Slack workspace"
          className={`${box} w-44`}
          workspaces={workspaces}
          value={editing.row.slack_workspace}
          onChange={(domain) => set("slack_workspace", domain)}
          onConnect={onConnectSlack}
          allowUnlisted
        />
      </td>
      <td className="py-2 pr-2">
        <input aria-label="Channel name" placeholder="team-reviews" className={`${box} w-36`} value={editing.row.slack_channel_name} onChange={(e) => set("slack_channel_name", e.target.value)} />
        <input aria-label="Channel id" placeholder="C0123456789" className={`${box} mt-1 w-36 font-mono text-xs`} value={editing.row.slack_channel_id} onChange={(e) => set("slack_channel_id", e.target.value)} />
      </td>
      <td className="py-2 pr-2">
        <label className="flex items-center gap-1 text-xs">
          <input type="checkbox" checked={editing.disabled} onChange={(e) => onChange({ ...editing, disabled: e.target.checked })} />
          Disabled
        </label>
      </td>
      <td className="py-2 text-right whitespace-nowrap">
        <button type="button" className="rounded bg-violet-500 px-2 py-1 text-xs" onClick={onSave}>
          Save
        </button>
        <button type="button" className="ml-1 rounded px-2 py-1 text-xs hover:bg-slate-800" onClick={onCancel}>
          Cancel
        </button>
        {error && (
          <p role="alert" className="mt-1 max-w-56 text-left text-xs whitespace-normal text-rose-300">
            {error}
          </p>
        )}
      </td>
    </tr>
  );
}

function ImportPanel({
  api,
  workspaces,
  onConnectSlack,
  onImported,
}: {
  api: Api;
  workspaces: SlackWorkspace[];
  onConnectSlack?: () => void;
  onImported: (rows: number) => void;
}) {
  const [csv, setCsv] = useState("");
  const [picked, setPicked] = useState("");
  const workspace = chosenWorkspace(workspaces, picked);
  // The backend needs a default only for rows without their own workspace.
  const needsWorkspace = csv.trim() !== "" && !workspace && !csvHasWorkspaceColumn(csv);
  const [mode, setMode] = useState<ImportMode>("merge");
  const [preview, setPreview] = useState<{ key: string; diff: ImportPreview } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const key = `${mode}\n${workspace}\n${csv}`;
  const current = preview?.key === key ? preview.diff : null;

  async function runPreview() {
    setError(null);
    try {
      setPreview({ key, diff: await api.previewMappingImport(csv, workspace, mode) });
    } catch (err) {
      setPreview(null);
      setError(errorMessage(err));
    }
  }

  async function apply() {
    setError(null);
    try {
      const { rows } = await api.applyMappingImport(csv, workspace, mode);
      rememberWorkspace(workspace);
      setCsv("");
      setPreview(null);
      onImported(rows);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <section aria-labelledby="mapping-import" className="space-y-3">
      <h2 id="mapping-import" className="text-base font-semibold">
        Import CSV
      </h2>
      <fieldset className="flex gap-4 text-sm">
        <legend className="sr-only">Import mode</legend>
        <label className="flex items-center gap-1">
          <input type="radio" name="import-mode" checked={mode === "merge"} onChange={() => setMode("merge")} />
          Merge into my edits
        </label>
        <label className="flex items-center gap-1">
          <input type="radio" name="import-mode" checked={mode === "replace"} onChange={() => setMode("replace")} />
          Replace the imported rows
        </label>
      </fieldset>
      <div className="text-sm">
        <span>Default Slack workspace</span>
        <span className="mt-0.5 block text-xs text-slate-500">
          Used for rows without a <code>slack_workspace</code> column.
        </span>
        <div className="mt-1">
          <WorkspaceSelect
            label="Default Slack workspace"
            className={`${box} w-80`}
            workspaces={workspaces}
            value={workspace}
            onChange={setPicked}
            onConnect={onConnectSlack}
            connectLabel="Connect Slack in Accounts"
          />
        </div>
      </div>
      <label className="block text-sm">
        Mapping CSV
        <textarea
          className={`${box} mt-1 h-28 w-full font-mono text-xs`}
          placeholder="github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id"
          value={csv}
          onChange={(e) => setCsv(e.target.value)}
        />
      </label>
      <label className="block text-xs text-slate-400">
        Or load a file
        <input
          type="file"
          accept=".csv,text/csv"
          className="ml-2 text-xs"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) {
              void file.text().then(setCsv);
            }
          }}
        />
      </label>
      <div className="flex items-center gap-2">
        <button type="button" className="rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800 disabled:opacity-50" disabled={!csv.trim() || needsWorkspace} onClick={() => void runPreview()}>
          Preview changes
        </button>
        <button type="button" className="rounded bg-violet-500 px-3 py-1.5 text-sm disabled:opacity-50" disabled={!current} onClick={() => void apply()}>
          Apply import
        </button>
      </div>
      {needsWorkspace && (
        <p className="text-xs text-amber-200">
          Connect a Slack workspace, or add a <code>slack_workspace</code> column to the CSV.
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-rose-300">
          {error}
        </p>
      )}
      {current && <DiffSummary diff={current} />}
    </section>
  );
}

function DiffSummary({ diff }: { diff: ImportPreview }) {
  const label = (row: MappingRow) => `${row.github_org}/${row.team_slug}`;
  return (
    <div role="status" className="rounded border border-slate-800 bg-slate-900/60 p-3 text-sm">
      <p>
        {diff.added.length} new · {diff.changed.length} changed · {diff.unchanged} unchanged
      </p>
      {diff.added.length > 0 && (
        <ul className="mt-2 list-disc pl-5 text-slate-300">
          {diff.added.map((row) => (
            <li key={label(row)}>
              Add {label(row)} → #{row.slack_channel_name || row.slack_channel_id}
            </li>
          ))}
        </ul>
      )}
      {diff.changed.length > 0 && (
        <ul className="mt-2 list-disc pl-5 text-slate-300">
          {diff.changed.map((changed) => (
            <li key={label(changed.row)}>
              {label(changed.row)}:{" "}
              {changed.changes
                .map((change) => `${MAPPING_FIELDS[change.field] ?? change.field} ${change.before || "—"} → ${change.after || "—"}`)
                .join(", ")}
            </li>
          ))}
        </ul>
      )}
      {diff.user_shadowed.length > 0 && (
        <p className="mt-2 text-amber-200">
          Your edits still win for {diff.user_shadowed.map(label).join(", ")}. Use Reset to imported to drop them.
        </p>
      )}
    </div>
  );
}

function ExportPanel({ api }: { api: Api }) {
  const [mode, setMode] = useState<ExportMode>("effective");
  const [csv, setCsv] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  return (
    <section aria-labelledby="mapping-export" className="space-y-3">
      <h2 id="mapping-export" className="text-base font-semibold">
        Export CSV
      </h2>
      <div className="flex items-center gap-2 text-sm">
        <label className="flex items-center gap-2">
          Rows
          <select className={box} value={mode} onChange={(e) => setMode(e.target.value as ExportMode)}>
            <option value="effective">What applies (edits over imports)</option>
            <option value="user">My edits only</option>
            <option value="imported">Imported only</option>
          </select>
        </label>
        <button
          type="button"
          className="rounded border border-slate-700 px-3 py-1.5 hover:bg-slate-800"
          onClick={() => {
            setStatus(null);
            api
              .exportMapping(mode)
              .then(setCsv)
              .catch((err: unknown) => setStatus(errorMessage(err)));
          }}
        >
          Export
        </button>
        {csv !== null && (
          <button
            type="button"
            className="rounded border border-slate-700 px-3 py-1.5 hover:bg-slate-800"
            onClick={() => {
              void navigator.clipboard
                ?.writeText(csv)
                .then(() => setStatus("Copied."))
                .catch(() => setStatus("Select the text and copy it."));
            }}
          >
            Copy
          </button>
        )}
        {status && (
          <span role="status" className="text-slate-300">
            {status}
          </span>
        )}
      </div>
      {csv !== null && <textarea aria-label="Exported CSV" readOnly className={`${box} h-28 w-full font-mono text-xs`} value={csv} />}
    </section>
  );
}
