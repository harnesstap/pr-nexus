import { plainText } from "../textInput";
import { useEffect, useState } from "react";
import type { Api, ProfileDto, ReuseFrom, ReuseItems, ReuseOptions } from "../api";
import { ChevronLeftIcon } from "../icons";
import { ProfileAvatar } from "../list/AccountMenu";

const NONE: ReuseItems = { github: false, slackTeams: [], templates: false, sendPolicy: false, projects: false };

/** Only the profiles something was picked from, in list order. */
export function reuseSelection(picks: Record<string, ReuseItems>, order: string[]): ReuseFrom[] {
  return order
    .map((profile) => ({ profile, items: picks[profile] ?? NONE }))
    .filter(({ items }) => items.github || items.slackTeams.length > 0 || items.templates || items.sendPolicy || items.projects);
}

interface Props {
  api: Api;
  profiles: ProfileDto[];
  /** The new profile exists and is active; the wizard takes over. */
  onCreated: (profile: ProfileDto) => void;
  onCancel: () => void;
}

/** First step of Add a profile: name it, and pick what to copy from existing profiles. */
export function ProfileReuse({ api, profiles, onCreated, onCancel }: Props) {
  const [name, setName] = useState("");
  const [options, setOptions] = useState<ReuseOptions[]>([]);
  const [picks, setPicks] = useState<Record<string, ReuseItems>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void Promise.all(profiles.map((profile) => api.profileReuseOptions?.(profile.id) ?? Promise.resolve(null))).then((found) => {
      if (live) {
        setOptions(found.filter((item): item is ReuseOptions => item !== null));
      }
    });
    return () => {
      live = false;
    };
  }, [api, profiles]);

  function update(profile: string, change: Partial<ReuseItems>) {
    setPicks((current) => ({ ...current, [profile]: { ...(current[profile] ?? NONE), ...change } }));
  }

  // One GitHub account per profile: picking it in one source clears the others.
  function pickGithub(profile: string, value: boolean) {
    setPicks((current) => {
      const next = { ...current };
      for (const id of Object.keys(next)) {
        next[id] = { ...next[id], github: false };
      }
      next[profile] = { ...(next[profile] ?? NONE), github: value };
      return next;
    });
  }

  async function create() {
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Give the profile a name.");
      return;
    }
    if (!api.createProfile) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const reuse = reuseSelection(
        picks,
        options.map((item) => item.profile),
      );
      onCreated(await api.createProfile(trimmed, reuse));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="mx-auto flex min-h-screen max-w-3xl flex-col px-6 py-8 text-slate-100">
      <header>
        <div className="flex items-center justify-between">
          <p className="text-sm text-slate-400">Add a profile</p>
          <button type="button" className="flex cursor-pointer items-center gap-1 rounded px-2 py-1 text-sm text-slate-300 hover:bg-slate-800" onClick={onCancel}>
            <ChevronLeftIcon />
            Back to PR Nexus
          </button>
        </div>
        <h1 className="mt-4 text-xl font-semibold">Start from what you already have</h1>
        <p className="mt-1 text-sm text-slate-400">
          A profile has its own GitHub account, Slack workspaces, projects and activity. Pick anything to copy from your other profiles; copies are independent, and setup
          continues with the rest.
        </p>
      </header>
      <form
        className="mt-6 flex flex-col gap-5"
        onSubmit={(event) => {
          event.preventDefault();
          void create();
        }}
      >
        <label className="flex flex-col gap-1 text-sm">
          <span className="text-slate-300">Profile name</span>
          <input {...plainText}
            autoFocus
            value={name}
            maxLength={40}
            placeholder="For example Work or Side projects"
            onChange={(event) => setName(event.target.value)}
            className="rounded border border-slate-700 bg-slate-900 px-3 py-2 text-slate-100 outline-none focus:border-violet-400"
          />
        </label>
        {options.map((option) => {
          const profile = profiles.find((item) => item.id === option.profile);
          const pick = picks[option.profile] ?? NONE;
          return (
            <fieldset key={option.profile} className="rounded-lg border border-slate-800 bg-slate-900/60 p-4">
              <legend className="flex items-center gap-2 px-1 text-sm font-medium text-slate-200">
                {profile && <ProfileAvatar profile={profile} size={20} />}
                Copy from {profile?.name ?? option.profile}
              </legend>
              <div className="mt-1 grid gap-2 text-sm text-slate-300">
                {option.githubLogin && (
                  <Check checked={pick.github} onChange={(value) => pickGithub(option.profile, value)}>
                    GitHub account <span className="text-slate-500">@{option.githubLogin}, with its tracked orgs</span>
                  </Check>
                )}
                {option.slackTeams.map((team) => (
                  <Check
                    key={team.team}
                    checked={pick.slackTeams.includes(team.team)}
                    onChange={(value) =>
                      update(option.profile, {
                        slackTeams: value ? [...pick.slackTeams, team.team] : pick.slackTeams.filter((item) => item !== team.team),
                      })
                    }
                  >
                    Slack workspace <span className="text-slate-500">{team.name}, with its channel mappings</span>
                  </Check>
                ))}
                {option.projects > 0 && (
                  <Check checked={pick.projects} onChange={(value) => update(option.profile, { projects: value })}>
                    Projects <span className="text-slate-500">{option.projects} with their rules</span>
                  </Check>
                )}
                {option.templates > 0 && (
                  <Check checked={pick.templates} onChange={(value) => update(option.profile, { templates: value })}>
                    Message templates
                  </Check>
                )}
                {option.sendPolicy && (
                  <Check checked={pick.sendPolicy} onChange={(value) => update(option.profile, { sendPolicy: value })}>
                    Agent send policy <span className="text-slate-500">allowed workspaces, auto-send channels, approvals</span>
                  </Check>
                )}
              </div>
            </fieldset>
          );
        })}
        {error && <p className="text-sm text-rose-300">{error}</p>}
        <div className="flex justify-end">
          <button type="submit" disabled={busy} className="cursor-pointer rounded bg-violet-500 px-4 py-2 text-sm font-medium text-white hover:bg-violet-400 disabled:opacity-60">
            {busy ? "Creating…" : "Create and continue setup"}
          </button>
        </div>
      </form>
    </main>
  );
}

function Check({ checked, onChange, children }: { checked: boolean; onChange: (value: boolean) => void; children: React.ReactNode }) {
  return (
    <label className="flex cursor-pointer items-center gap-2">
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} className="accent-violet-400" />
      <span>{children}</span>
    </label>
  );
}
