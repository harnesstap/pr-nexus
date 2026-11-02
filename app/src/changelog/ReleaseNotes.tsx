import type { ChangelogRelease } from "./changelog";

/** One release: heading with version and date, then entries grouped by kind. `collapsed` folds it into a <details>. */
export function ReleaseNotes({ release, current, collapsed }: { release: ChangelogRelease; current?: boolean; collapsed?: boolean }) {
  if (collapsed) {
    const count = release.sections.reduce((total, section) => total + section.entries.length, 0);
    return (
      <details className={`rounded border ${current ? "border-violet-500/60 bg-violet-500/10" : "border-slate-800 bg-slate-900/60"}`}>
        <summary className="px-3 py-2 text-slate-300 hover:text-slate-100">
          <span className="font-semibold text-slate-100">v{release.version}</span>
          {release.date && <span className="ml-2 text-xs text-slate-400">{release.date}</span>}
          <span className="ml-2 text-xs text-slate-500">
            {count} {count === 1 ? "change" : "changes"}
          </span>
          {current && <span className="ml-2 rounded bg-violet-500/30 px-1.5 py-0.5 text-[11px] font-medium text-violet-100">Installed</span>}
        </summary>
        <div className="border-t border-slate-800">
          <ReleaseNotes release={release} current={current} />
        </div>
      </details>
    );
  }
  const id = `release-${release.version ?? "unreleased"}`;
  return (
    <section
      aria-labelledby={id}
      data-current={current || undefined}
      className={`rounded border p-3 ${current ? "border-violet-500/60 bg-violet-500/10" : "border-slate-800 bg-slate-900/60"}`}
    >
      <div className="flex flex-wrap items-baseline gap-2">
        <h3 id={id} className="text-sm font-semibold text-slate-100">
          {release.version ? `v${release.version}` : "Unreleased"}
        </h3>
        {release.date && <span className="text-xs text-slate-400">{release.date}</span>}
        {current && <span className="rounded bg-violet-500/30 px-1.5 py-0.5 text-[11px] font-medium text-violet-100">Installed</span>}
        {!release.version && <span className="text-xs text-slate-400">Not in a release yet</span>}
      </div>
      {release.sections.map((section) => (
        <div key={section.kind} className="mt-2">
          <h4 className="text-xs font-semibold uppercase tracking-wide text-slate-400">{section.kind}</h4>
          <ul className="mt-1 list-disc space-y-1 pl-5 text-slate-200">
            {section.entries.map((entry, index) => (
              <li key={index}>{entry}</li>
            ))}
          </ul>
        </div>
      ))}
    </section>
  );
}
