import { useEffect, useState } from "react";
import type { Api } from "../api";
import { compareVersions, type ChangelogRelease } from "./changelog";
import { RELEASES, UNRELEASED } from "./data";
import { ReleaseNotes } from "./ReleaseNotes";

interface Props {
  api: Pick<Api, "appInfo" | "openExternal">;
  releases?: ChangelogRelease[];
  unreleased?: ChangelogRelease | null;
  /** How many released versions to list. */
  limit?: number;
}

/** Settings → Changelog: recent releases embedded in this build, newest first (§8.2). */
export function ChangelogSettings({ api, releases = RELEASES, unreleased = UNRELEASED, limit = 10 }: Props) {
  const [info, setInfo] = useState<{ version: string; repository: string } | null>(null);
  useEffect(() => {
    let cancel = false;
    api
      .appInfo()
      .then((next) => !cancel && setInfo(next))
      .catch(() => undefined);
    return () => {
      cancel = true;
    };
  }, [api]);

  const shown = releases.slice(0, limit);
  const releasesUrl = info?.repository ? `https://github.com/${info.repository}/releases` : null;
  return (
    <div className="space-y-4 text-sm">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-base font-semibold">Recent changes</h2>
        {releasesUrl && (
          <a
            className="text-violet-300 underline hover:text-violet-200"
            href={releasesUrl}
            onClick={(event) => {
              event.preventDefault();
              void api.openExternal(releasesUrl);
            }}
          >
            All releases on GitHub ↗
          </a>
        )}
      </div>
      {unreleased && <ReleaseNotes release={unreleased} />}
      {shown.length === 0 && !unreleased && <p className="text-slate-400">No release notes in this build.</p>}
      {shown.map((release, index) => {
        const current = !!info && compareVersions(release.version!, info.version) === 0;
        // The newest release and the installed one stay open; older ones fold to keep the tab short.
        return <ReleaseNotes key={release.version} release={release} current={current} collapsed={index > 0 && !current} />;
      })}
    </div>
  );
}
