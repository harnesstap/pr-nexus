import { useEffect, useState } from "react";
import type { Api, AppInfo, UpdateInfo } from "../api";
import { errorMessage } from "../api";

interface Props {
  api: Api;
  onRerunSetup: () => void;
}

type UpdateState = { phase: "idle" | "checking" | "current" } | { phase: "available"; update: UpdateInfo } | { phase: "error"; message: string };

/** Settings → About: version, paths, the keychain explainer, update check, Re-run setup. */
export function AboutSettings({ api, onRerunSetup }: Props) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateState>({ phase: "idle" });
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    let cancel = false;
    api
      .appInfo()
      .then((next) => {
        if (!cancel) {
          setInfo(next);
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

  function copy(label: string, text: string) {
    void navigator.clipboard
      ?.writeText(text)
      .then(() => setCopied(label))
      .catch(() => setCopied(null));
  }

  function checkUpdate() {
    setUpdate({ phase: "checking" });
    api
      .checkUpdate()
      .then((next) => setUpdate(next ? { phase: "available", update: next } : { phase: "current" }))
      .catch((err: unknown) => setUpdate({ phase: "error", message: errorMessage(err) }));
  }

  if (error) {
    return (
      <p role="alert" className="text-sm text-rose-300">
        Could not read app details: {error}
      </p>
    );
  }
  if (!info) {
    return <p className="text-sm text-slate-400">Loading…</p>;
  }

  const isMac = info.os === "macos";
  return (
    <div className="space-y-8 text-sm">
      <section aria-labelledby="about-app">
        <h2 id="about-app" className="text-base font-semibold">
          PR-Nexus {info.version}
        </h2>
        <dl className="mt-3 grid grid-cols-[9rem_1fr_auto] items-center gap-x-3 gap-y-2">
          {info.repository && <PathRow label="Repository" value={`https://github.com/${info.repository}`} onCopy={copy} />}
          <PathRow label="Database" value={info.db_path} onCopy={copy} />
          <PathRow label="Logs" value={info.log_dir} onCopy={copy} />
        </dl>
        <p className="mt-2 text-xs text-slate-500">
          Logs rotate daily and keep a week. Tokens, OAuth codes and secrets are replaced with [REDACTED] before a line is
          written.
        </p>
        {copied && (
          <p role="status" className="mt-1 text-xs text-slate-300">
            Copied {copied}.
          </p>
        )}
      </section>

      <section aria-labelledby="about-updates">
        <h2 id="about-updates" className="text-base font-semibold">
          Updates
        </h2>
        <div className="mt-2 flex items-center gap-3">
          <button
            type="button"
            className="rounded border border-slate-700 px-3 py-1.5 hover:bg-slate-800 disabled:opacity-50"
            disabled={update.phase === "checking"}
            onClick={checkUpdate}
          >
            {update.phase === "checking" ? "Checking…" : "Check for updates"}
          </button>
          <span role="status">
            {update.phase === "current" && "You have the latest release."}
            {update.phase === "error" && <span className="text-rose-300">{update.message}</span>}
            {update.phase === "available" && (
              <>
                {update.update.version} is available.{" "}
                <a className="text-violet-300 underline" href={update.update.url} target="_blank" rel="noreferrer">
                  Release notes and download
                </a>
              </>
            )}
          </span>
        </div>
        <p className="mt-2 text-xs text-slate-500">
          PR-Nexus asks GitHub for the latest release with your token; it never downloads or installs anything itself.
        </p>
        {isMac && (
          <div className="mt-3 rounded border border-slate-800 bg-slate-900/60 p-3 text-xs text-slate-300">
            <p className="font-medium text-slate-200">Installing an unsigned build on macOS</p>
            <p className="mt-1">Download the DMG from the release page, or with the GitHub CLI:</p>
            <code className="mt-1 block font-mono">gh release download --pattern &apos;*.dmg&apos; -R {info.repository || "OWNER/REPO"}</code>
            <p className="mt-1">
              Drag PR-Nexus to Applications. The first launch is blocked: open System Settings → Privacy &amp; Security and
              click Open Anyway.
            </p>
          </div>
        )}
      </section>

      <section aria-labelledby="about-keychain">
        <h2 id="about-keychain" className="text-base font-semibold">
          {isMac ? "Keychain prompts" : "Credential store"}
        </h2>
        {info.keyring_in_memory ? (
          <p role="alert" className="mt-2 text-amber-200">
            The system credential store is unavailable, so tokens live in memory only and you will sign in again after a
            restart.
          </p>
        ) : isMac ? (
          <p className="mt-2 text-slate-300">
            PR-Nexus keeps every token in one keychain item. Builds are not signed yet, so after each update macOS sees a new
            app and asks once for access to that item. Choose Always Allow; the prompt does not come back until the next
            update.
          </p>
        ) : (
          <p className="mt-2 text-slate-300">PR-Nexus keeps every token in one entry of the system credential store.</p>
        )}
      </section>

      <section aria-labelledby="about-setup">
        <h2 id="about-setup" className="text-base font-semibold">
          Setup
        </h2>
        <p className="mt-2 text-slate-300">
          Walk through the setup steps again to reconnect GitHub or Slack, change the tracked orgs, or import a mapping. Your
          PRs, projects and settings stay as they are.
        </p>
        <button type="button" className="mt-2 rounded border border-slate-700 px-3 py-1.5 hover:bg-slate-800" onClick={onRerunSetup}>
          Re-run setup
        </button>
      </section>
    </div>
  );
}

function PathRow({ label, value, onCopy }: { label: string; value: string; onCopy: (label: string, text: string) => void }) {
  return (
    <>
      <dt className="text-slate-400">{label}</dt>
      <dd className="truncate font-mono text-xs" title={value}>
        {value}
      </dd>
      <dd>
        <button type="button" aria-label={`Copy ${label.toLowerCase()}`} className="rounded px-2 py-0.5 text-xs hover:bg-slate-800" onClick={() => onCopy(label.toLowerCase(), value)}>
          Copy
        </button>
      </dd>
    </>
  );
}
