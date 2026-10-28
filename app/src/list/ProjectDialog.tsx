import { useEffect, useRef, useState } from "react";
import { CloseButton } from "../CloseButton";
import type { Api, JiraLinkDto, ListRow, ProjectDto, ProjectLinkDto, ProjectRuleDto, RuleKind } from "../api";
import { errorMessage } from "../api";
import { JiraChip } from "./JiraChips";
import { PROJECT_COLORS, RULE_KINDS, ruleKindLabel } from "./projects";
import { plainText } from "../textInput";
import { Tooltip } from "../Tooltip";

/** Candidate search waits this long after the last keystroke. */
const CANDIDATE_DEBOUNCE_MS = 200;
const CANDIDATE_LIMIT = 8;

interface Props {
  api: Api;
  project: ProjectDto;
  onClose: () => void;
  /** Rows or projects changed; the list should reload. */
  onChanged: () => Promise<void>;
}

type Draft = ProjectRuleDto & { key: string; error: string | null };

/**
 * Project management (§8.2): color, auto-link rules with a live preview, and the
 * linked PRs with their source. Manual links are added here or by dragging rows.
 */
export function ProjectDialog({ api, project, onClose, onChanged }: Props) {
  const [rules, setRules] = useState<Draft[]>([]);
  const [links, setLinks] = useState<ProjectLinkDto[]>([]);
  const [query, setQuery] = useState("");
  const [byId, setById] = useState<Map<number, ListRow>>(() => new Map());
  const [candidates, setCandidates] = useState<ListRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const closeRef = useRef<HTMLButtonElement>(null);

  async function load() {
    const [nextRules, nextLinks] = await Promise.all([api.listProjectRules(project.id), api.listProjectLinks(project.id)]);
    setRules(nextRules.map((rule) => ({ ...rule, key: `rule-${rule.id}`, error: null })));
    setLinks(nextLinks);
  }

  useEffect(() => {
    let cancel = false;
    void Promise.all([api.listProjectRules(project.id), api.listProjectLinks(project.id)])
      .then(([nextRules, nextLinks]) => {
        if (!cancel) {
          setRules(nextRules.map((rule) => ({ ...rule, key: `rule-${rule.id}`, error: null })));
          setLinks(nextLinks);
        }
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setError(errorMessage(err));
        }
      });
    closeRef.current?.focus();
    return () => {
      cancel = true;
    };
  }, [api, project.id]);

  // Titles for the linked PRs, in one call however many there are.
  useEffect(() => {
    let cancel = false;
    const ids = links.map((link) => link.pr_id);
    if (ids.length === 0) {
      setById(new Map());
      return;
    }
    void api.rowsById(ids).then((found) => !cancel && setById(new Map(found.map((row) => [row.id, row]))));
    return () => {
      cancel = true;
    };
  }, [api, links]);

  // Backend search over every open PR; a stale answer for an older query is dropped.
  useEffect(() => {
    const needle = query.trim();
    if (!needle) {
      setCandidates([]);
      return;
    }
    let cancel = false;
    const timer = window.setTimeout(() => {
      void api
        .linkCandidates(project.id, needle, CANDIDATE_LIMIT)
        .then((found) => !cancel && setCandidates(found))
        .catch((err: unknown) => !cancel && setError(errorMessage(err)));
    }, CANDIDATE_DEBOUNCE_MS);
    return () => {
      cancel = true;
      window.clearTimeout(timer);
    };
  }, [api, project.id, query, links]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function run(action: () => Promise<unknown>) {
    setError(null);
    try {
      await action();
      await load();
      await onChanged();
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function saveRule(draft: Draft) {
    try {
      await api.saveProjectRule({
        id: draft.id,
        project_id: project.id,
        kind: draft.kind,
        pattern: draft.pattern,
        priority: draft.priority,
      });
      await load();
      await onChanged();
    } catch (err) {
      setRules((current) => current.map((rule) => (rule.key === draft.key ? { ...rule, error: errorMessage(err) } : rule)));
    }
  }

  const ruleById = new Map(rules.filter((rule) => rule.id !== null).map((rule) => [rule.id, rule]));

  return (
    <div className="fixed inset-0 z-30 flex items-start justify-center overflow-auto bg-black/50 p-8" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="project-dialog-title"
        className="w-full max-w-3xl rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl"
      >
        <header className="flex items-center justify-between gap-4">
          <h2 id="project-dialog-title" className="flex items-center gap-2 text-base font-semibold">
            <span aria-hidden="true" className="h-3 w-3 rounded-sm" style={{ background: project.color }} />
            {project.name}
          </h2>
          <CloseButton ref={closeRef} onClick={onClose} />
        </header>

        <fieldset className="mt-4">
          <legend className="text-xs font-medium uppercase tracking-wide text-slate-500">Color</legend>
          <div className="mt-1 flex gap-1.5">
            {PROJECT_COLORS.map((color) => (
              <button
                key={color}
                type="button"
                aria-label={`Color ${color}`}
                aria-pressed={project.color === color}
                className={`h-5 w-5 rounded ${project.color === color ? "ring-2 ring-white" : ""}`}
                style={{ background: color }}
                onClick={() => void run(() => api.setProjectColor(project.id, color))}
              />
            ))}
          </div>
        </fieldset>

        <DescriptionField api={api} project={project} onSaved={onChanged} />
        <JiraLinksEditor api={api} project={project} onSaved={onChanged} />

        <section aria-labelledby="rules-heading" className="mt-5">
          <div className="flex items-center justify-between">
            <h3 id="rules-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
              Auto-link rules
            </h3>
            <button
              type="button"
              className="rounded bg-slate-800 px-2 py-1 text-xs hover:bg-slate-700"
              onClick={() =>
                setRules((current) => [
                  ...current,
                  { id: null, project_id: project.id, kind: "ticket_key_in_branch", pattern: "", priority: 0, key: `new-${Date.now()}`, error: null },
                ])
              }
            >
              Add rule
            </button>
          </div>
          <p className="mt-1 text-xs text-slate-500">
            Rules run on every sync. The highest priority match wins; PRs you remove stay excluded.
          </p>
          {rules.length === 0 && <p className="mt-2 text-xs text-slate-500">No rules. PRs join this project only when you add them.</p>}
          <ul className="mt-2 space-y-2">
            {rules.map((draft) => (
              <RuleRow
                key={draft.key}
                api={api}
                draft={draft}
                onChange={(next) => setRules((current) => current.map((rule) => (rule.key === draft.key ? { ...next, error: null } : rule)))}
                onSave={() => void saveRule(draft)}
                onDelete={() =>
                  draft.id === null
                    ? setRules((current) => current.filter((rule) => rule.key !== draft.key))
                    : void run(() => api.deleteProjectRule(draft.id!))
                }
              />
            ))}
          </ul>
        </section>

        <section aria-labelledby="linked-heading" className="mt-5">
          <h3 id="linked-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
            Linked pull requests ({links.filter((link) => link.source !== "excluded").length})
          </h3>
          <label className="mt-2 flex items-center gap-2">
            <span className="sr-only">Add pull request</span>
            <input {...plainText}
              aria-label="Add pull request"
              className="w-full rounded border border-slate-700 bg-slate-950 px-2 py-1"
              placeholder="Add a pull request: search by title, repo or #number"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && candidates[0]) {
                  event.preventDefault();
                  const first = candidates[0];
                  void run(() => api.linkPrs(project.id, [first.id])).then(() => setQuery(""));
                }
              }}
            />
          </label>
          {query.trim() && (
            <ul aria-label="Matching pull requests" className="mt-1 rounded border border-slate-800">
              {candidates.length === 0 && <li className="px-2 py-1.5 text-xs text-slate-500">No matching PRs outside this project.</li>}
              {candidates.map((row) => (
                <li key={row.id} className="flex items-center justify-between gap-2 border-t border-slate-800 px-2 py-1 first:border-t-0">
                  <span className="truncate">
                    <span className="text-slate-500">{row.repo}#{row.number}</span> {row.title}
                  </span>
                  <button
                    type="button"
                    aria-label={`Add ${row.title}`}
                    className="rounded bg-slate-800 px-2 py-0.5 text-xs hover:bg-slate-700"
                    onClick={() => void run(() => api.linkPrs(project.id, [row.id]))}
                  >
                    Add
                  </button>
                </li>
              ))}
            </ul>
          )}
          <ul className="mt-2 divide-y divide-slate-800 rounded border border-slate-800">
            {links.length === 0 && <li className="px-2 py-2 text-xs text-slate-500">No linked PRs yet. Drag rows onto the project, or add one above.</li>}
            {links.map((link) => {
              const row = byId.get(link.pr_id);
              const rule = link.rule_id !== null ? ruleById.get(link.rule_id) : undefined;
              const title = row ? row.title : `Pull request ${link.pr_id} (not in the list)`;
              return (
                <li key={link.pr_id} className={`flex items-center gap-2 px-2 py-1.5 ${link.source === "excluded" ? "text-slate-500" : ""}`}>
                  <span className="min-w-0 flex-1 truncate">
                    {row && <span className="text-slate-500">{row.repo}#{row.number} </span>}
                    {title}
                  </span>
                  <SourceBadge source={link.source} rule={rule} />
                  {link.source === "excluded" ? (
                    <Tooltip label="Let auto-link rules add this PR again" align="end">
                      <button
                        type="button"
                        aria-label={`Restore ${title}`}
                        className="rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                        onClick={() => void run(() => api.restorePr(project.id, link.pr_id))}
                      >
                        Restore
                      </button>
                    </Tooltip>
                  ) : (
                    <Tooltip label={link.source === "rule" ? "Removes it and keeps rules from adding it back" : "Remove from this project"} align="end">
                      <button
                        type="button"
                        aria-label={`Remove ${title}`}
                        className="rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                        onClick={() => void run(() => api.unlinkPr(project.id, link.pr_id))}
                      >
                        Remove
                      </button>
                    </Tooltip>
                  )}
                </li>
              );
            })}
          </ul>
        </section>
        {error && <p className="mt-3 text-xs text-rose-300">{error}</p>}
      </div>
    </div>
  );
}

/** The multi-line description: saved on blur, ⌘/Ctrl+Enter or Save. */
function DescriptionField({ api, project, onSaved }: { api: Api; project: ProjectDto; onSaved: () => Promise<void> }) {
  const [saved, setSaved] = useState(project.description);
  const [draft, setDraft] = useState(project.description);
  const [error, setError] = useState<string | null>(null);
  const dirty = draft !== saved;

  async function save() {
    if (!dirty) {
      return;
    }
    try {
      const next = await api.setProjectDescription(project.id, draft);
      setSaved(next.description);
      setDraft(next.description);
      setError(null);
      await onSaved();
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <section aria-labelledby="description-heading" className="mt-5">
      <div className="flex items-center justify-between">
        <h3 id="description-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
          Description
        </h3>
        <button type="button" aria-label="Save description" className="rounded bg-slate-800 px-2 py-1 text-xs hover:bg-slate-700 disabled:opacity-40" disabled={!dirty} onClick={() => void save()}>
          Save
        </button>
      </div>
      <textarea
        {...plainText}
        aria-label="Project description"
        rows={3}
        className="mt-2 w-full resize-y rounded border border-slate-700 bg-slate-950 px-2 py-1"
        placeholder="What this project is about. Shown with the project in the list."
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => void save()}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void save();
          }
        }}
      />
      {error && <p className="mt-1 text-xs text-rose-300">{error}</p>}
    </section>
  );
}

/** How a saved link goes back to the backend when the list is rewritten. */
function linkText(link: JiraLinkDto): string {
  return link.url || link.key;
}

/** JIRA references: add a key or a link, remove, and reorder with ↑/↓. */
function JiraLinksEditor({ api, project, onSaved }: { api: Api; project: ProjectDto; onSaved: () => Promise<void> }) {
  const [links, setLinks] = useState<JiraLinkDto[]>(project.jira_links);
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function apply(action: () => Promise<JiraLinkDto[]>) {
    try {
      setLinks(await action());
      setError(null);
      await onSaved();
      return true;
    } catch (err) {
      setError(errorMessage(err));
      return false;
    }
  }

  async function add() {
    const text = input.trim();
    if (!text) {
      return;
    }
    const added = await apply(async () => [...links, await api.addJiraLink(project.id, { text })]);
    if (added) {
      setInput("");
    }
  }

  function move(index: number, by: number) {
    const next = [...links];
    const [item] = next.splice(index, 1);
    next.splice(index + by, 0, item);
    void apply(() => api.setJiraLinks(project.id, next.map((link) => ({ text: linkText(link) }))));
  }

  return (
    <section aria-labelledby="jira-heading" className="mt-5">
      <h3 id="jira-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
        JIRA links
      </h3>
      <form
        className="mt-2 flex items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void add();
        }}
      >
        <input
          {...plainText}
          aria-label="Add JIRA key or link"
          className="min-w-0 flex-1 rounded border border-slate-700 bg-slate-950 px-2 py-1"
          placeholder="ACME-123 or https://acme-labs.atlassian.net/browse/ACME-123"
          value={input}
          onChange={(event) => {
            setInput(event.target.value);
            setError(null);
          }}
        />
        <button type="submit" className="rounded bg-slate-800 px-2 py-1 text-xs hover:bg-slate-700 disabled:opacity-40" disabled={!input.trim()}>
          Add
        </button>
      </form>
      {error && <p className="mt-1 text-xs text-rose-300">{error}</p>}
      {links.length === 0 ? (
        <p className="mt-2 text-xs text-slate-500">No JIRA links. A bare key links out once a JIRA base URL is set in Settings.</p>
      ) : (
        <ul aria-label="JIRA links" className="mt-2 divide-y divide-slate-800 rounded border border-slate-800">
          {links.map((link, index) => {
            const name = link.key || link.url;
            return (
              <li key={link.id} className="flex items-center gap-2 px-2 py-1">
                <JiraChip link={link} />
                <span className="min-w-0 flex-1 truncate text-xs text-slate-500">{link.url || (link.href ? "" : "No JIRA base URL set: not a link yet")}</span>
                <button type="button" aria-label={`Move ${name} up`} className="rounded px-1.5 text-xs hover:bg-slate-800 disabled:opacity-30" disabled={index === 0} onClick={() => move(index, -1)}>
                  ↑
                </button>
                <button type="button" aria-label={`Move ${name} down`} className="rounded px-1.5 text-xs hover:bg-slate-800 disabled:opacity-30" disabled={index === links.length - 1} onClick={() => move(index, 1)}>
                  ↓
                </button>
                <button
                  type="button"
                  aria-label={`Remove ${name}`}
                  className="rounded px-2 py-0.5 text-xs hover:bg-slate-800"
                  onClick={() =>
                    void apply(async () => {
                      await api.removeJiraLink(link.id);
                      return links.filter((item) => item.id !== link.id);
                    })
                  }
                >
                  Remove
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function SourceBadge({ source, rule }: { source: ProjectLinkDto["source"]; rule: ProjectRuleDto | undefined }) {
  const text = source === "manual" ? "Manual" : source === "excluded" ? "Excluded" : "Rule";
  const title = source === "rule" && rule ? `${ruleKindLabel(rule.kind)}: ${rule.pattern}` : undefined;
  const tone = source === "manual" ? "border-sky-500/40 text-sky-200" : source === "rule" ? "border-violet-500/40 text-violet-200" : "border-slate-700 text-slate-500";
  return (
    <Tooltip label={title} className="shrink-0">
      <span className={`shrink-0 rounded border px-1.5 text-xs ${tone}`}>{text}</span>
    </Tooltip>
  );
}

function RuleRow({
  api,
  draft,
  onChange,
  onSave,
  onDelete,
}: {
  api: Api;
  draft: Draft;
  onChange: (draft: Draft) => void;
  onSave: () => void;
  onDelete: () => void;
}) {
  const preview = usePreview(api, draft.kind, draft.pattern);
  const kind = RULE_KINDS.find((item) => item.id === draft.kind) ?? RULE_KINDS[0];
  return (
    <li className="rounded border border-slate-800 p-2">
      <form
        className="flex flex-wrap items-center gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          onSave();
        }}
      >
        <select
          aria-label="Rule kind"
          className="rounded border border-slate-700 bg-slate-950 px-1 py-1"
          value={draft.kind}
          onChange={(event) => onChange({ ...draft, kind: event.target.value as RuleKind })}
        >
          {RULE_KINDS.map((item) => (
            <option key={item.id} value={item.id}>
              {item.label}
            </option>
          ))}
        </select>
        <input {...plainText}
          aria-label="Rule pattern"
          className="min-w-40 flex-1 rounded border border-slate-700 bg-slate-950 px-2 py-1 font-mono text-xs"
          placeholder={kind.placeholder}
          value={draft.pattern}
          onChange={(event) => onChange({ ...draft, pattern: event.target.value })}
        />
        <label className="flex items-center gap-1 text-xs text-slate-400">
          Priority
          <input {...plainText}
            aria-label="Rule priority"
            type="number"
            className="w-16 rounded border border-slate-700 bg-slate-950 px-1 py-1"
            value={draft.priority}
            onChange={(event) => onChange({ ...draft, priority: Number.parseInt(event.target.value, 10) || 0 })}
          />
        </label>
        <button type="submit" className="rounded bg-violet-500 px-2 py-1 text-xs text-white">
          Save
        </button>
        <button type="button" aria-label="Delete rule" className="rounded px-2 py-1 text-xs hover:bg-slate-800" onClick={onDelete}>
          Delete
        </button>
      </form>
      <p className={`mt-1 text-xs ${draft.error || preview.error ? "text-rose-300" : "text-slate-500"}`} aria-live="polite">
        {draft.error ?? preview.error ?? (preview.count === null ? kind.hint : `Matches ${preview.count} open ${preview.count === 1 ? "PR" : "PRs"}`)}
      </p>
    </li>
  );
}

/** Debounced live preview of how many open PRs a pattern matches. */
function usePreview(api: Api, kind: RuleKind, pattern: string): { count: number | null; error: string | null } {
  const [result, setResult] = useState<{ key: string; count: number | null; error: string | null }>({ key: "", count: null, error: null });
  const key = `${kind}\u0000${pattern}`;
  useEffect(() => {
    if (!pattern.trim()) {
      return;
    }
    let cancel = false;
    const timer = window.setTimeout(() => {
      api
        .previewRule(kind, pattern)
        .then((count) => !cancel && setResult({ key, count, error: null }))
        .catch((err: unknown) => !cancel && setResult({ key, count: null, error: errorMessage(err) }));
    }, 250);
    return () => {
      cancel = true;
      window.clearTimeout(timer);
    };
  }, [api, kind, pattern, key]);
  if (!pattern.trim() || result.key !== key) {
    return { count: null, error: null };
  }
  return result;
}
