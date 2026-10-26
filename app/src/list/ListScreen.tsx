import { useEffect, useState, type ReactNode } from "react";
import type { Api, Bootstrap, ListRow, ProjectDto } from "../api";
import { errorMessage } from "../api";
import { ClockIcon, FolderIcon, GearIcon, HomeIcon, LogoIcon, PlusIcon, RefreshIcon } from "../icons";
import { OrgOption } from "../orgs";
import { DRAG_MIME, TOAST_MS, bulkToast, defaultView, parseDragIds, parseViewState, renameError, sentToast, syncLabel, type SavedView, type Toast } from "./model";
import { PrList } from "./PrList";
import { PrDetailDialog } from "./PrDetailDialog";
import { ProjectDialog } from "./ProjectDialog";
import { SlackSettings } from "../slack/SlackSettings";
import { BulkDialog } from "../slack/BulkDialog";
import { ReviewDialog } from "../slack/ReviewDialog";

interface Props {
  api: Api;
  boot: Bootstrap;
  onBoot: (boot: Bootstrap) => void;
}

export function ListScreen({ api, boot, onBoot }: Props) {
  const [rows, setRows] = useState<ListRow[]>([]);
  const [projects, setProjects] = useState<ProjectDto[]>([]);
  const [viewKey, setViewKey] = useState("mine");
  const [views, setViews] = useState<Record<string, SavedView>>({});
  const [loaded, setLoaded] = useState<string | null>(null);
  const [sync, setSync] = useState(boot.sync);
  const [detail, setDetail] = useState(false);
  const [now, setNow] = useState(() => new Date());
  const [orgs, setOrgs] = useState(boot.tracked_orgs);
  const [personal, setPersonal] = useState(boot.include_personal);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<{ name: string; error: string | null } | null>(null);
  const [editingProject, setEditingProject] = useState<number | null>(null);
  const [detailId, setDetailId] = useState<number | null>(null);
  const [reviewId, setReviewId] = useState<number | null>(null);
  const [bulkIds, setBulkIds] = useState<number[] | null>(null);
  const [notice, setNotice] = useState<Toast | null>(null);
  /** The bulk run's summary, shown as a toast once its dialog closes. */
  const [bulkSummary, setBulkSummary] = useState<string | null>(null);
  const view = views[viewKey] ?? defaultView();
  const needsOrgs = orgs.length === 0 && !personal;

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!notice) {
      return;
    }
    const timer = window.setTimeout(() => setNotice(null), TOAST_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);

  useEffect(() => {
    let cancel = false;
    void Promise.all([api.listRows(), api.listProjects()])
      .then(([nextRows, nextProjects]) => {
        if (!cancel) {
          setRows(nextRows);
          setProjects(nextProjects);
        }
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setError(errorMessage(err));
        }
      });
    return () => {
      cancel = true;
    };
  }, [api]);

  useEffect(() => {
    let cancel = false;
    void api.loadViewState(viewKey).then((json) => {
      if (cancel) {
        return;
      }
      setViews((current) => ({ ...current, [viewKey]: parseViewState(json) }));
      setLoaded(viewKey);
    });
    return () => {
      cancel = true;
    };
  }, [api, viewKey]);

  useEffect(() => {
    if (loaded !== viewKey) {
      return;
    }
    const timer = window.setTimeout(() => {
      void api.saveViewState(viewKey, JSON.stringify(view));
    }, 200);
    return () => window.clearTimeout(timer);
  }, [api, view, viewKey, loaded]);

  async function reload() {
    setRows(await api.listRows());
    setProjects(await api.listProjects());
  }

  async function refresh() {
    setError(null);
    setSync({ ...sync, phase: "syncing", message: "Syncing…" });
    try {
      const status = await api.syncNow();
      setSync(status);
      await reload();
      onBoot(await api.bootstrap());
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function createProject() {
    if (!draft) {
      return;
    }
    const problem = renameError(draft.name, projects, null);
    if (problem) {
      setDraft({ ...draft, error: problem });
      return;
    }
    try {
      const project = await api.createProject(draft.name.trim());
      setDraft(null);
      setProjects(await api.listProjects());
      setViewKey(`project:${project.id}`);
    } catch (err) {
      setDraft({ ...draft, error: errorMessage(err) });
    }
  }

  async function linkPrs(projectId: number, prIds: number[]) {
    setError(null);
    const name = projects.find((project) => project.id === projectId)?.name ?? "project";
    try {
      const changed = await api.linkPrs(projectId, prIds);
      await reload();
      const count = `${prIds.length} ${prIds.length === 1 ? "PR" : "PRs"}`;
      setNotice({ text: changed > 0 ? `Added ${count} to ${name}` : `${count} already in ${name}` });
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  const activeProjects = projects.filter((project) => !project.archived);
  const dialogProject = projects.find((project) => project.id === editingProject) ?? null;
  const detailRow = rows.find((row) => row.id === detailId) ?? null;
  const reviewRow = rows.find((row) => row.id === reviewId) ?? null;
  const syncTone = sync.phase === "error" || sync.paused || sync.phase === "paused" ? "bg-amber-400" : sync.phase === "syncing" ? "bg-sky-400" : "bg-emerald-400";

  async function saveOrgs() {
    setError(null);
    try {
      await api.saveOrgs(orgs, personal);
      await refresh();
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <div className="flex h-screen bg-slate-950 text-slate-100">
      <aside className="flex w-60 shrink-0 flex-col border-r border-slate-800 bg-slate-950">
        <p className="flex items-center gap-2 px-4 py-4 text-base font-semibold">
          <LogoIcon size={20} className="text-violet-400" />
          PR-Nexus
        </p>
        <nav aria-label="Views" className="flex flex-col gap-0.5 px-2 text-sm">
          <NavButton current={viewKey} id="mine" icon={<HomeIcon />} onSelect={setViewKey}>
            My PRs
          </NavButton>
          <NavButton current={viewKey} id="to_review" icon={<ClockIcon />} onSelect={setViewKey}>
            To review
          </NavButton>
        </nav>
        <section aria-labelledby="projects-heading" className="mt-4 flex min-h-0 flex-1 flex-col px-2 text-sm">
          <div className="flex items-center justify-between px-2 pb-1">
            <h2 id="projects-heading" className="flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-slate-500">
              <FolderIcon size={14} />
              Projects
            </h2>
            <button
              type="button"
              aria-label="New project"
              title="New project"
              className="rounded p-0.5 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
              onClick={() => setDraft({ name: "", error: null })}
            >
              <PlusIcon size={14} />
            </button>
          </div>
          <div className="flex min-h-0 flex-col gap-0.5 overflow-auto">
            {activeProjects.map((project) => (
              <NavButton
                key={project.id}
                current={viewKey}
                id={`project:${project.id}`}
                icon={<span aria-hidden="true" className="ml-1 mr-0.5 h-2 w-2 rounded-sm" style={{ background: project.color }} />}
                onSelect={setViewKey}
                onDropPrs={(ids) => void linkPrs(project.id, ids)}
              >
                {project.name}
              </NavButton>
            ))}
          </div>
          {draft ? (
            <form
              className="mt-1 px-1"
              onSubmit={(event) => {
                event.preventDefault();
                void createProject();
              }}
            >
              <input
                aria-label="Project name"
                autoFocus
                className="w-full rounded border border-slate-600 bg-slate-900 px-2 py-1 text-sm"
                placeholder="Project name"
                value={draft.name}
                onChange={(event) => setDraft({ name: event.target.value, error: null })}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    setDraft(null);
                  }
                }}
              />
              {draft.error ? (
                <p className="mt-1 text-xs text-rose-300">{draft.error}</p>
              ) : (
                <p className="mt-1 text-xs text-slate-500">Enter to create · Esc to cancel</p>
              )}
            </form>
          ) : (
            activeProjects.length === 0 && (
              <div className="mx-1 mt-1 rounded-md border border-dashed border-slate-800 px-3 py-3 text-xs text-slate-500">
                <p>No projects yet. Group related PRs, for example by feature or initiative.</p>
                <button
                  type="button"
                  className="mt-2 flex items-center gap-1 rounded bg-slate-800 px-2 py-1 text-slate-200 hover:bg-slate-700"
                  onClick={() => setDraft({ name: "", error: null })}
                >
                  <PlusIcon size={12} />
                  Create project
                </button>
              </div>
            )
          )}
        </section>
        <footer className="border-t border-slate-800 px-2 py-2 text-sm">
          <NavButton current={viewKey} id="settings" icon={<GearIcon />} onSelect={setViewKey}>
            Settings
          </NavButton>
          <div className="mt-1 flex items-center justify-between gap-2 px-2 text-xs text-slate-400">
            <button type="button" className="flex items-center gap-2 text-left" onClick={() => setDetail((open) => !open)}>
              <span aria-hidden="true" className={`h-2 w-2 rounded-full ${syncTone}`} />
              {syncLabel(sync, now)}
            </button>
            <button type="button" aria-label="Refresh" title="Sync now" className="rounded p-1 hover:bg-slate-800 hover:text-slate-200" onClick={() => void refresh()}>
              <RefreshIcon size={14} />
            </button>
          </div>
          {detail && <p className="px-2 pt-1 text-xs text-slate-500">{sync.message}</p>}
        </footer>
      </aside>
      <main className="relative flex min-w-0 flex-1 flex-col">
        {boot.keyring_warning && (
          <p className="border-b border-amber-500/40 bg-amber-500/10 px-4 py-2 text-sm text-amber-100">{boot.keyring_warning}</p>
        )}
        {viewKey === "settings" || needsOrgs ? (
          <section className="max-w-xl px-6 py-8">
            <h1 className="text-lg font-semibold">Choose orgs to track</h1>
            <p className="mt-2 text-sm text-slate-400">
              Nothing is selected by default. The list stays empty until you pick at least one org or include personal repos.
            </p>
            <ul className="mt-4 space-y-2">
              {boot.org_choices.map((org) => (
                <li key={org}>
                  <OrgOption
                    org={org}
                    checked={orgs.includes(org)}
                    needsSso={boot.orgs_needing_sso.includes(org)}
                    onToggle={() =>
                      setOrgs((current) => (current.includes(org) ? current.filter((item) => item !== org) : [...current, org]))
                    }
                    onOpen={(url) => void api.openExternal(url)}
                  />
                </li>
              ))}
            </ul>
            <label className="mt-4 flex items-center gap-2">
              <input type="checkbox" checked={personal} onChange={(event) => setPersonal(event.target.checked)} />
              Include my personal repos
            </label>
            <button type="button" className="mt-4 rounded bg-violet-500 px-3 py-2 text-sm" onClick={() => void saveOrgs()}>
              Save orgs
            </button>
            {viewKey === "settings" && <SlackSettings api={api} boot={boot} />}
          </section>
        ) : (
          <PrList
            rows={rows}
            projects={projects}
            viewKey={viewKey}
            view={view}
            now={now}
            onViewChange={(next) => setViews((current) => ({ ...current, [viewKey]: next }))}
            onOpen={(url) => void api.openExternal(url)}
            onRename={async (id, name) => {
              await api.renameProject(id, name);
              await reload();
            }}
            onArchive={(id) => {
              const project = projects.find((item) => item.id === id);
              void api.archiveProject(id, !(project?.archived ?? false)).then(reload);
            }}
            onDelete={(id) => {
              void api.deleteProject(id).then(reload);
            }}
            onEditProject={setEditingProject}
            onLinkPrs={(projectId, ids) => void linkPrs(projectId, ids)}
            onDetails={(row) => setDetailId(row.id)}
            onRequestReview={(row) => setReviewId(row.id)}
            onBulkReview={setBulkIds}
          />
        )}
        {notice && (
          <p
            role="status"
            className="fixed bottom-6 right-6 z-50 flex items-center gap-3 rounded border border-emerald-700 bg-slate-900 px-4 py-2 text-sm text-slate-100 shadow-lg"
          >
            {notice.text}
            {notice.link && (
              <button type="button" className="text-violet-300 underline" onClick={() => void api.openExternal(notice.link!)}>
                Open ↗
              </button>
            )}
            <button type="button" aria-label="Dismiss" className="text-slate-500 hover:text-slate-200" onClick={() => setNotice(null)}>
              ×
            </button>
          </p>
        )}
        {error && <p className="px-4 py-2 text-sm text-rose-300">{error}</p>}
        {detailRow && (
          <PrDetailDialog key={detailRow.id} api={api} row={detailRow} onClose={() => {
              setDetailId(null);
              // The detail may have resolved code-owner status the list row now uses.
              void reload();
            }}
            onOpen={(url) => void api.openExternal(url)}
            onRequestReview={() => {
              setDetailId(null);
              setReviewId(detailRow.id);
            }} />
        )}
        {reviewRow && (
          <ReviewDialog
            key={reviewRow.id}
            api={api}
            row={reviewRow}
            onClose={() => setReviewId(null)}
            onOpen={(url) => void api.openExternal(url)}
            onSent={(slack) => {
              const id = reviewRow.id;
              setRows((current) => current.map((row) => (row.id === id ? { ...row, slack } : row)));
              setReviewId(null);
              setNotice(sentToast(slack));
            }}
          />
        )}
        {bulkIds && (
          <BulkDialog
            api={api}
            prIds={bulkIds}
            onClose={() => {
              setBulkIds(null);
              if (bulkSummary) {
                setNotice({ text: bulkSummary });
                setBulkSummary(null);
              }
            }}
            onOpen={(url) => void api.openExternal(url)}
            onSent={(done) => {
              setBulkSummary(bulkToast(done));
              const byId = new Map(done.indicators.map((item) => [item.pr_id, item.indicator]));
              setRows((current) => current.map((row) => (byId.has(row.id) ? { ...row, slack: byId.get(row.id) ?? null } : row)));
            }}
          />
        )}
        {dialogProject && (
          <ProjectDialog api={api} project={dialogProject} rows={rows} onClose={() => setEditingProject(null)} onChanged={reload} />
        )}
      </main>
    </div>
  );
}

function NavButton({
  current,
  id,
  icon,
  onSelect,
  onDropPrs,
  children,
}: {
  current: string;
  id: string;
  icon: ReactNode;
  onSelect: (id: string) => void;
  /** Makes the entry a drop target for dragged PR rows (manual project links). */
  onDropPrs?: (ids: number[]) => void;
  children: string;
}) {
  const [over, setOver] = useState(false);
  const selected = current === id;
  const tone = over ? "bg-violet-500/20 text-white ring-1 ring-violet-400" : selected ? "bg-slate-800 text-white" : "text-slate-300 hover:bg-slate-900";
  return (
    <button
      type="button"
      aria-current={selected ? "page" : undefined}
      className={`flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left ${tone}`}
      onClick={() => onSelect(id)}
      onDragOver={
        onDropPrs &&
        ((event) => {
          if (event.dataTransfer.types.includes(DRAG_MIME)) {
            event.preventDefault();
            event.dataTransfer.dropEffect = "link";
            setOver(true);
          }
        })
      }
      onDragLeave={onDropPrs && (() => setOver(false))}
      onDrop={
        onDropPrs &&
        ((event) => {
          setOver(false);
          const ids = parseDragIds(event.dataTransfer.getData(DRAG_MIME));
          if (ids.length > 0) {
            event.preventDefault();
            onDropPrs(ids);
          }
        })
      }
    >
      <span className="flex w-4 shrink-0 justify-center text-slate-400">{icon}</span>
      <span className="truncate">{children}</span>
    </button>
  );
}

