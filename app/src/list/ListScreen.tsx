import { useEffect, useState } from "react";
import type { Api, Bootstrap, ListRow, ProjectDto } from "../api";
import { errorMessage } from "../api";
import { defaultView, parseViewState, syncLabel, type SavedView } from "./model";
import { PrList } from "./PrList";

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
  const view = views[viewKey] ?? defaultView();
  const needsOrgs = orgs.length === 0 && !personal;

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

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
      <aside className="flex w-56 shrink-0 flex-col border-r border-slate-800">
        <p className="px-4 py-4 text-sm font-semibold">PR-Nexus</p>
        <nav className="flex flex-col gap-1 px-2 text-sm">
          <NavButton current={viewKey} id="mine" onSelect={setViewKey}>
            My PRs
          </NavButton>
          <NavButton current={viewKey} id="to_review" onSelect={setViewKey}>
            To review
          </NavButton>
          <p className="mt-3 px-2 text-xs uppercase tracking-wide text-slate-500">Projects</p>
          {projects
            .filter((project) => !project.archived)
            .map((project) => (
              <NavButton key={project.id} current={viewKey} id={`project:${project.id}`} onSelect={setViewKey}>
                {project.name}
              </NavButton>
            ))}
          <NavButton current={viewKey} id="settings" onSelect={setViewKey}>
            Settings
          </NavButton>
        </nav>
        <footer className="mt-auto flex items-center justify-between gap-2 border-t border-slate-800 px-3 py-3 text-xs text-slate-400">
          <button type="button" className="text-left" onClick={() => setDetail((open) => !open)}>
            {syncLabel(sync, now)}
          </button>
          <button type="button" aria-label="Refresh" onClick={() => void refresh()}>
            ⟳
          </button>
        </footer>
        {detail && <p className="px-3 pb-3 text-xs text-slate-500">{sync.message}</p>}
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
                  <label className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      checked={orgs.includes(org)}
                      onChange={() =>
                        setOrgs((current) => (current.includes(org) ? current.filter((item) => item !== org) : [...current, org]))
                      }
                    />
                    {org}
                  </label>
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
          />
        )}
        {error && <p className="px-4 py-2 text-sm text-rose-300">{error}</p>}
      </main>
    </div>
  );
}

function NavButton({
  current,
  id,
  onSelect,
  children,
}: {
  current: string;
  id: string;
  onSelect: (id: string) => void;
  children: string;
}) {
  const selected = current === id;
  return (
    <button
      type="button"
      aria-current={selected ? "page" : undefined}
      className={`rounded px-2 py-1.5 text-left ${selected ? "bg-slate-800 text-white" : "text-slate-300"}`}
      onClick={() => onSelect(id)}
    >
      {children}
    </button>
  );
}
