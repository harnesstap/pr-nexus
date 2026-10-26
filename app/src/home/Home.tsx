import { useEffect, useState } from "react";
import type { Api, Bootstrap, ListRow } from "../api";
import { errorMessage } from "../api";

interface Props {
  api: Api;
  boot: Bootstrap;
  onBoot: (boot: Bootstrap) => void;
}

export function Home({ api, boot, onBoot }: Props) {
  const [rows, setRows] = useState<ListRow[]>([]);
  const [orgs, setOrgs] = useState(boot.tracked_orgs);
  const [personal, setPersonal] = useState(boot.include_personal);
  const [showOrgs, setShowOrgs] = useState(boot.tracked_orgs.length === 0 && !boot.include_personal);
  const [syncMessage, setSyncMessage] = useState(boot.sync.message);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api.listRows().then(setRows).catch((err: unknown) => setError(errorMessage(err)));
  }, [api]);

  const needsOrgs = orgs.length === 0 && !personal;
  const visible = rows.filter((row) => !row.hidden);

  async function save() {
    setError(null);
    try {
      await api.saveOrgs(orgs, personal);
      const status = await api.syncNow();
      setSyncMessage(status.message);
      setRows(await api.listRows());
      setShowOrgs(false);
      onBoot(await api.bootstrap());
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function refresh() {
    setError(null);
    try {
      const status = await api.syncNow();
      setSyncMessage(status.message);
      setRows(await api.listRows());
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <main className="min-h-screen bg-slate-950 text-slate-100">
      <div className="mx-auto max-w-3xl px-6 py-10">
        {boot.keyring_warning && (
          <p className="mb-4 rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-100">
            {boot.keyring_warning}
          </p>
        )}
        <header className="flex items-center justify-between">
          <h1 className="text-2xl font-semibold">Pull requests</h1>
          <button type="button" className="text-sm text-slate-300" onClick={() => void refresh()}>
            Refresh
          </button>
        </header>
        <p className="mt-2 text-sm text-slate-400">{syncMessage}</p>
        {needsOrgs || showOrgs ? (
          <section className="mt-8 rounded-lg border border-slate-800 p-6">
            <h2 className="text-lg font-medium">Choose orgs to track</h2>
            <p className="mt-2 text-sm text-slate-400">
              Nothing is selected by default. PR-Nexus stays empty until you pick at least one org
              or include personal repos.
            </p>
            <ul className="mt-4 space-y-2">
              {boot.org_choices.map((org) => (
                <li key={org}>
                  <label className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      checked={orgs.includes(org)}
                      onChange={() =>
                        setOrgs((current) =>
                          current.includes(org)
                            ? current.filter((item) => item !== org)
                            : [...current, org],
                        )
                      }
                    />
                    {org}
                  </label>
                </li>
              ))}
            </ul>
            <label className="mt-4 flex items-center gap-2">
              <input
                type="checkbox"
                checked={personal}
                onChange={(event) => setPersonal(event.target.checked)}
              />
              Include my personal repos
            </label>
            <button
              type="button"
              className="mt-4 rounded bg-violet-500 px-3 py-2 text-sm font-medium"
              onClick={() => void save()}
            >
              Save orgs
            </button>
          </section>
        ) : (
          <section className="mt-8">
            <p className="text-sm text-slate-400">{visible.length} open pull requests in the cache</p>
            <ul className="mt-4 divide-y divide-slate-800">
              {visible.map((row) => (
                <li key={row.id} className="py-3">
                  <p className="font-medium">
                    #{row.number} {row.title}
                  </p>
                  <p className="text-sm text-slate-400">{row.repo}</p>
                </li>
              ))}
            </ul>
            <button type="button" className="mt-6 text-sm text-violet-300" onClick={() => setShowOrgs(true)}>
              Settings · tracked orgs
            </button>
          </section>
        )}
        {error && <p className="mt-4 text-sm text-rose-300">{error}</p>}
      </div>
    </main>
  );
}
