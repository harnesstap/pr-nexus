import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { Api, Bootstrap, ListRow, ProjectDto, ReplacedLinkDto } from "../api";
import { errorMessage } from "../api";
import { ClockIcon, CloseIcon, FolderIcon, GearIcon, HomeIcon, LogoIcon, PlusIcon, RefreshIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { OrgOption } from "../orgs";
import { DRAG_MIME, TOAST_MS, bulkToast, defaultView, parseDragIds, parseViewState, renameError, sentToast, syncLabel, type SavedView, type Toast } from "./model";
import { PrList, type ListActions } from "./PrList";
import { GlobalCommands } from "./GlobalCommands";
import type { PageRequest } from "./paging";

/** Sync status poll while syncing. The backend updates the count at most this often too. */
const SYNC_POLL_MS = 500;
/** Idle poll that notices scheduled background syncs (§6). */
const IDLE_POLL_MS = 15_000;
import { DetailPanel } from "../detail/DetailPanel";
import { ProjectDialog } from "./ProjectDialog";
import { AccountsSettings } from "../settings/AccountsSettings";
import { BulkDialog } from "../slack/BulkDialog";
import { ReviewDialog } from "../slack/ReviewDialog";
import { SettingsScreen } from "../settings/SettingsScreen";
import type { SettingsTab } from "../settings/tabs";
import { KeyringBanner } from "../KeyringBanner";
import { SlackSignInBanner, SyncErrorBanner, SyncPopover } from "./SyncBanners";
import { MappingHint, UpdateBanner } from "./Banners";
import { showMappingHint } from "./hints";
import { plainText } from "../textInput";

interface Props {
  api: Api;
  boot: Bootstrap;
  onBoot: (boot: Bootstrap) => void;
  /** Settings → About → Re-run setup. */
  onRerunSetup?: () => void;
}

export function ListScreen({ api, boot, onBoot, onRerunSetup }: Props) {
  /** Bumped whenever stored rows may have changed; the list re-reads in place. */
  const [version, setVersion] = useState(0);
  const [projects, setProjects] = useState<ProjectDto[]>([]);
  const [viewKey, setViewKey] = useState("mine");
  const [views, setViews] = useState<Record<string, SavedView>>({});
  const [loaded, setLoaded] = useState<string | null>(null);
  const [sync, setSync] = useState(boot.sync);
  // The last sync revision the list was read at: re-read only when it moves,
  // so a sync that changed nothing costs no list query (§6).
  const revision = useRef(boot.sync.revision);
  const noteRevision = useCallback((next: number) => {
    if (next !== revision.current) {
      revision.current = next;
      setVersion((value) => value + 1);
    }
  }, []);
  const [detail, setDetail] = useState(false);
  const [now, setNow] = useState(() => new Date());
  const [orgs, setOrgs] = useState(boot.tracked_orgs);
  const [personal, setPersonal] = useState(boot.include_personal);
  const [error, setError] = useState<string | null>(null);
  /** First load of the list: null while loading, then "ready" or the error. */
  const [listState, setListState] = useState<{ phase: "loading" } | { phase: "ready" } | { phase: "error"; message: string }>({ phase: "loading" });
  const [attempt, setAttempt] = useState(0);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("accounts");
  const [mappingHint, setMappingHint] = useState(() => showMappingHint(boot.mapping_count, boot.slack_workspaces.length));
  const [draft, setDraft] = useState<{ name: string; error: string | null; withSelection: boolean } | null>(null);
  /** The list's current selection, offered to the sidebar's new-project field. */
  const [selection, setSelection] = useState<number[]>([]);
  /** A sidebar project to scroll to and highlight after PRs were added to it. */
  const [flash, setFlash] = useState<number | null>(null);
  const [editingProject, setEditingProject] = useState<number | null>(null);
  const [detailRow, setDetailRow] = useState<ListRow | null>(null);
  const [reviewRow, setReviewRow] = useState<ListRow | null>(null);
  const closeDetail = () => {
    setDetailRow(null);
    // The detail may have resolved code-owner status the list row now uses.
    void reload();
  };
  const [bulkIds, setBulkIds] = useState<number[] | null>(null);
  const listActions = useRef<ListActions | null>(null);
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
    if (flash === null) {
      return;
    }
    document.querySelector(`[data-project-id="${flash}"]`)?.scrollIntoView?.({ block: "nearest" });
    const timer = window.setTimeout(() => setFlash(null), 1_500);
    return () => window.clearTimeout(timer);
  }, [flash, projects]);

  useEffect(() => {
    if (!notice) {
      return;
    }
    const timer = window.setTimeout(() => setNotice(null), TOAST_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);

  // Sync runs in the background (§8.2): poll its status, and re-read the list
  // while it writes so new rows show up without blocking the first screen.
  const syncing = sync.phase === "syncing";
  useEffect(() => {
    if (!syncing) {
      return;
    }
    let cancel = false;
    const timer = window.setInterval(() => {
      void api.syncStatus().then((status) => {
        if (cancel) {
          return;
        }
        setSync(status);
        noteRevision(status.revision);
        if (status.phase !== "syncing") {
          void api.listProjects().then((next) => !cancel && setProjects(next));
        }
      });
    }, SYNC_POLL_MS);
    return () => {
      cancel = true;
      window.clearInterval(timer);
    };
  }, [api, syncing, noteRevision]);

  // Between syncs, poll slowly so a scheduled background sync that changed
  // rows shows up, and so its "Syncing…" phase starts the fast poll above.
  useEffect(() => {
    if (syncing) {
      return;
    }
    let cancel = false;
    const timer = window.setInterval(() => {
      void api.syncStatus().then((status) => {
        if (cancel) {
          return;
        }
        if (status.phase === "syncing" || status.revision !== revision.current) {
          setSync(status);
        }
        noteRevision(status.revision);
      });
    }, IDLE_POLL_MS);
    return () => {
      cancel = true;
      window.clearInterval(timer);
    };
  }, [api, syncing, noteRevision]);

  const [openCount, setOpenCount] = useState<number | null>(null);
  const loadPage = useCallback(
    async (request: PageRequest) => {
      const page = await api.listPage(request);
      setOpenCount(page.summary.open);
      return page;
    },
    [api],
  );

  useEffect(() => {
    let cancel = false;
    void api
      .listProjects()
      .then((nextProjects) => {
        if (!cancel) {
          setProjects(nextProjects);
          setListState({ phase: "ready" });
        }
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setListState({ phase: "error", message: errorMessage(err) });
        }
      });
    return () => {
      cancel = true;
    };
  }, [api, attempt]);

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

  // Optimistic draft flips, kept until the reload after the save has landed.
  const [draftPatches, setDraftPatches] = useState<Record<number, { draft: boolean; until: number }>>({});
  const shownDraftPatches = useMemo(() => {
    const live: Record<number, boolean> = {};
    for (const [id, patch] of Object.entries(draftPatches)) if (version <= patch.until) live[Number(id)] = patch.draft;
    return live;
  }, [draftPatches, version]);

  function changeDraft(id: number, draft: boolean | null | "saved") {
    if (draft === "saved") {
      setDraftPatches((current) => (current[id] ? { ...current, [id]: { ...current[id], until: version + 1 } } : current));
      void reload();
      return;
    }
    if (draft === null) {
      setDraftPatches((current) => {
        const rest = { ...current };
        delete rest[id];
        return rest;
      });
      setDetailRow((current) => (current && current.id === id ? { ...current, is_draft: !current.is_draft } : current));
      return;
    }
    setDraftPatches((current) => ({ ...current, [id]: { draft, until: Number.MAX_SAFE_INTEGER } }));
    setDetailRow((current) => (current && current.id === id ? { ...current, is_draft: draft } : current));
  }

  async function reload() {
    setVersion((value) => value + 1);
    setProjects(await api.listProjects());
  }

  async function refresh(full = false) {
    setError(null);
    setSync({ ...sync, phase: "syncing", message: "Syncing…" });
    try {
      const status = await (full ? api.fullResync() : api.syncNow());
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
    const prIds = draft.withSelection ? selection : [];
    try {
      const name = draft.name.trim();
      const project = prIds.length > 0 ? await api.createProjectWithPrs(name, prIds) : await api.createProject(name);
      setDraft(null);
      if (prIds.length > 0) {
        await reload();
        setNotice({ text: `Created ${project.name} with ${prCount(prIds.length)}` });
      } else {
        setProjects(await api.listProjects());
      }
      setViewKey(`project:${project.id}`);
    } catch (err) {
      setDraft({ ...draft, error: errorMessage(err) });
    }
  }

  async function linkPrs(projectId: number, prIds: number[]) {
    setError(null);
    const name = projects.find((project) => project.id === projectId)?.name ?? "project";
    try {
      const outcome = await api.linkPrs(projectId, prIds);
      await reload();
      const count = prCount(prIds.length);
      setNotice(
        outcome.changed > 0
          ? { text: `Added ${count} to ${name}`, undo: () => void undoLinkPrs(projectId, name, outcome.undo) }
          : { text: `${count} already in ${name}` },
      );
      setFlash(projectId);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function undoLinkPrs(projectId: number, name: string, undo: ReplacedLinkDto[]) {
    setError(null);
    setNotice(null);
    try {
      const reverted = await api.undoLinkPrs(projectId, undo);
      await reload();
      setNotice({ text: reverted > 0 ? `Undid adding ${prCount(reverted)} to ${name}` : `Nothing to undo in ${name}` });
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  /** "Add to project → New project from selection…": one transaction, then a toast. */
  async function createProjectWithPrs(name: string, prIds: number[]) {
    setError(null);
    const project = await api.createProjectWithPrs(name, prIds);
    await reload();
    setNotice({ text: `Created ${project.name} with ${prCount(prIds.length)}` });
    setFlash(project.id);
  }

  const activeProjects = projects.filter((project) => !project.archived);
  const dialogProject = projects.find((project) => project.id === editingProject) ?? null;
  const syncTone = sync.phase === "error" ? "bg-rose-400" : sync.paused || sync.phase === "paused" ? "bg-amber-400" : sync.phase === "syncing" ? "bg-sky-400" : "bg-emerald-400";

  const orgPicker = () => (
    <div>
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
    </div>
  );

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
          PR Nexus
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
            <Tooltip label="New project" detail="Group related PRs, for example by feature or initiative" align="end">
              <button
                type="button"
                aria-label="New project"
                className="rounded p-0.5 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
                onClick={() => setDraft({ name: "", error: null, withSelection: selection.length > 0 })}
              >
                <PlusIcon />
              </button>
            </Tooltip>
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
                projectId={project.id}
                flash={flash === project.id}
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
              <input {...plainText}
                aria-label="Project name"
                autoFocus
                className="w-full rounded border border-slate-600 bg-slate-900 px-2 py-1 text-sm"
                placeholder="Project name"
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value, error: null })}
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    setDraft(null);
                  }
                }}
              />
              {selection.length > 0 && (
                <label className="mt-1 flex items-center gap-2 text-xs text-slate-300">
                  <input
                    type="checkbox"
                    checked={draft.withSelection}
                    onChange={(event) => setDraft({ ...draft, withSelection: event.target.checked })}
                  />
                  Add {selection.length} selected {selection.length === 1 ? "PR" : "PRs"}
                </label>
              )}
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
                  onClick={() => setDraft({ name: "", error: null, withSelection: selection.length > 0 })}
                >
                  <PlusIcon />
                  Create project
                </button>
              </div>
            )
          )}
        </section>
        <footer className="relative border-t border-slate-800 px-2 py-2 text-sm">
          <NavButton current={viewKey} id="settings" icon={<GearIcon />} onSelect={setViewKey}>
            Settings
          </NavButton>
          <div className="mt-1 flex items-center justify-between gap-2 px-2 text-xs text-slate-400">
            <button
              type="button"
              aria-expanded={detail}
              data-sync-trigger=""
              aria-label={`${syncLabel(sync, now)}, sync details`}
              className="flex items-center gap-2 text-left"
              onClick={() => setDetail((open) => !open)}
            >
              <span aria-hidden="true" className={`h-2 w-2 rounded-full ${syncTone}`} />
              {syncLabel(sync, now)}
            </button>
            <Tooltip label="Sync now" detail="Fetch the latest PRs, reviews and checks from GitHub" side="top" align="end">
              <button type="button" aria-label="Refresh" className="rounded p-1 hover:bg-slate-800 hover:text-slate-200" onClick={() => void refresh()}>
                <RefreshIcon />
              </button>
            </Tooltip>
          </div>
          {detail && (
            <SyncPopover
              sync={sync}
              now={now}
              prCount={openCount}
              onSync={() => void refresh()}
              onFullResync={() => void refresh(true)}
              onClose={() => setDetail(false)}
            />
          )}
        </footer>
      </aside>
      <main className="relative flex min-w-0 flex-1 flex-col">
        <UpdateBanner api={api} />
        {mappingHint && viewKey !== "settings" && (
          <MappingHint
            onOpen={() => {
              setSettingsTab("mappings");
              setViewKey("settings");
            }}
            onDismiss={() => setMappingHint(false)}
          />
        )}
        <SyncErrorBanner sync={sync} now={now} onReconnect={() => onRerunSetup?.()} onRetry={() => void refresh()} />
        {viewKey !== "settings" && (
          <SlackSignInBanner
            workspaces={boot.slack_workspaces}
            onReconnect={() => {
              setSettingsTab("accounts");
              setViewKey("settings");
            }}
          />
        )}
        <KeyringBanner api={api} initial={boot.keyring} onReady={() => void api.bootstrap().then(onBoot)} />
        {viewKey === "settings" ? (
          <section className="min-h-0 flex-1 overflow-auto px-6 py-8">
            <h1 className="mx-auto mb-4 max-w-3xl text-lg font-semibold">Settings</h1>
            <SettingsScreen
              api={api}
              boot={boot}
              onBoot={(next) => {
                onBoot(next);
                setMappingHint(showMappingHint(next.mapping_count, next.slack_workspaces.length));
              }}
              tab={settingsTab}
              onTabChange={setSettingsTab}
              onRerunSetup={() => onRerunSetup?.()}
              accounts={
                <AccountsSettings
                  api={api}
                  boot={boot}
                  onOrgsSaved={async (nextOrgs, nextPersonal) => {
                    setOrgs(nextOrgs);
                    setPersonal(nextPersonal);
                    await refresh();
                  }}
                  onMentionsChange={(scope) => onBoot({ ...boot, slack_mentions: scope })}
                  onWorkspacesChange={(workspaces) => onBoot({ ...boot, slack_workspaces: workspaces })}
                />
              }
            />
          </section>
        ) : needsOrgs ? (
          <section className="max-w-xl px-6 py-8">{orgPicker()}</section>
        ) : listState.phase === "loading" ? (
          <p role="status" className="px-6 py-8 text-sm text-slate-400">
            Loading pull requests…
          </p>
        ) : listState.phase === "error" ? (
          <div role="alert" className="px-6 py-8 text-sm">
            <p className="font-medium text-rose-200">Could not load your pull requests.</p>
            <p className="mt-1 text-slate-400">{listState.message}</p>
            <button
              type="button"
              className="mt-3 rounded border border-slate-700 px-3 py-1.5 hover:bg-slate-800"
              onClick={() => {
                setListState({ phase: "loading" });
                setAttempt((value) => value + 1);
              }}
            >
              Try again
            </button>
          </div>
        ) : (
          <div className="flex min-h-0 flex-1">
            <div className="flex min-w-0 flex-1 flex-col">
            <PrList
              loadPage={loadPage}
              version={version}
              syncProgress={syncing ? syncLabel(sync, now) : null}
              projects={projects}
              viewKey={viewKey}
              view={view}
              now={now}
              onViewChange={(next) => setViews((current) => ({ ...current, [viewKey]: next }))}
              onShowView={setViewKey}
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
              onCreateProjectWithPrs={createProjectWithPrs}
              onSelectionChange={setSelection}
              detailId={detailRow?.id ?? null}
              onDetails={setDetailRow}
              onRequestReview={setReviewRow}
              onBulkReview={setBulkIds}
              draftPatches={shownDraftPatches}
              actionsRef={listActions}
            />
            </div>
            {detailRow && (
              <DetailPanel
                key={detailRow.id}
                api={api}
                row={detailRow}
                version={version}
                onClose={closeDetail}
                onOpen={(url) => void api.openExternal(url)}
                onRequestReview={() => setReviewRow(detailRow)}
                onNotice={(text) => setNotice({ text })}
                onDraftChange={changeDraft}
                slack={{ mentions: boot.slack_mentions, workspaces: boot.slack_workspaces }}
              />
            )}
          </div>
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
            {notice.undo && (
              <button type="button" className="font-medium text-violet-300 hover:text-violet-200" onClick={notice.undo}>
                Undo
              </button>
            )}
            <Tooltip label="Dismiss" side="top" align="end">
              <button type="button" aria-label="Dismiss" className="rounded p-0.5 text-slate-500 hover:bg-slate-800 hover:text-slate-200" onClick={() => setNotice(null)}>
                <CloseIcon />
              </button>
            </Tooltip>
          </p>
        )}
        {error && <p className="px-4 py-2 text-sm text-rose-300">{error}</p>}
        {reviewRow && (
          <ReviewDialog
            key={reviewRow.id}
            api={api}
            row={reviewRow}
            onClose={() => setReviewRow(null)}
            onOpen={(url) => void api.openExternal(url)}
            onSent={(slack) => {
              setReviewRow(null);
              setVersion((value) => value + 1);
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
              setVersion((value) => value + 1);
            }}
          />
        )}
        <GlobalCommands
          api={api}
          projects={projects}
          actions={listActions}
          listShown={viewKey !== "settings"}
          onView={setViewKey}
          onDetails={setDetailRow}
          onSync={(full) => void refresh(full)}
          syncing={syncing}
        />
        {dialogProject && (
          <ProjectDialog api={api} project={dialogProject} onClose={() => setEditingProject(null)} onChanged={reload} />
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
  projectId,
  flash = false,
  children,
}: {
  current: string;
  id: string;
  icon: ReactNode;
  onSelect: (id: string) => void;
  /** Makes the entry a drop target for dragged PR rows (manual project links). */
  onDropPrs?: (ids: number[]) => void;
  /** Lets "Add to project" scroll to this entry. */
  projectId?: number;
  /** Briefly highlighted after PRs were added to the project. */
  flash?: boolean;
  children: string;
}) {
  const [over, setOver] = useState(false);
  const selected = current === id;
  const tone = over || flash ? "bg-violet-500/20 text-white ring-1 ring-violet-400" : selected ? "bg-slate-800 text-white" : "text-slate-300 hover:bg-slate-900";
  return (
    <button
      type="button"
      aria-current={selected ? "page" : undefined}
      data-project-id={projectId}
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

function prCount(count: number): string {
  return `${count} ${count === 1 ? "PR" : "PRs"}`;
}
