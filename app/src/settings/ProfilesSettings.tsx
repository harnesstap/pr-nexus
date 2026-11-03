import { plainText } from "../textInput";
import { useEffect, useState } from "react";
import type { Api, Bootstrap, ProfileDto } from "../api";
import { errorMessage } from "../api";
import { ProfileAvatar } from "../list/AccountMenu";
import { Tooltip } from "../Tooltip";

interface Props {
  api: Api;
  boot: Bootstrap;
  onBoot: (boot: Bootstrap) => void;
}

/** Why a profile can't be deleted, or null when it can. */
export function deleteBlocker(
  profile: ProfileDto,
  count: number,
): string | null {
  if (count <= 1) {
    return "The last profile can't be deleted.";
  }
  if (profile.isDefault) {
    return "The default profile holds the shared settings and can't be deleted.";
  }
  if (profile.active) {
    return "Switch to another profile first.";
  }
  return null;
}

/** Settings → Profiles: rename, and delete after a confirmation. */
export function ProfilesSettings({ api, boot, onBoot }: Props) {
  const [profiles, setProfiles] = useState<ProfileDto[]>(boot.profiles ?? []);
  const [editing, setEditing] = useState<{ id: string; name: string } | null>(
    null,
  );
  const [confirming, setConfirming] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancel = false;
    void api.listProfiles?.().then((next) => !cancel && setProfiles(next));
    return () => {
      cancel = true;
    };
  }, [api]);

  async function run(work: () => Promise<ProfileDto[]> | undefined) {
    setError(null);
    try {
      const next = await work();
      if (next) {
        setProfiles(next);
        onBoot({ ...boot, profiles: next });
      }
      return true;
    } catch (err) {
      setError(errorMessage(err));
      return false;
    }
  }

  return (
    <section aria-labelledby="settings-profiles" className="space-y-3">
      <div>
        <h2 id="settings-profiles" className="text-base font-semibold">
          Profiles
        </h2>
        <p className="mt-1 text-sm text-slate-400">
          Each profile has its own GitHub account, Slack workspaces, projects,
          templates, agent send policy and activity. Theme and other preferences
          are shared. Switch or add profiles from the account menu at the bottom
          left.
        </p>
      </div>
      <ul className="divide-y divide-slate-800 rounded-lg border border-slate-800">
        {profiles.map((profile) => {
          const blocker = deleteBlocker(profile, profiles.length);
          return (
            <li key={profile.id} className="px-3 py-2.5">
              <div className="flex items-center gap-3">
                <ProfileAvatar profile={profile} />
                {editing?.id === profile.id ? (
                  <form
                    className="flex flex-1 items-center gap-2"
                    onSubmit={(event) => {
                      event.preventDefault();
                      void run(() =>
                        api.renameProfile?.(profile.id, editing.name.trim()),
                      ).then((ok) => ok && setEditing(null));
                    }}
                  >
                    <input {...plainText}
                      autoFocus
                      aria-label="Profile name"
                      maxLength={40}
                      value={editing.name}
                      onChange={(event) =>
                        setEditing({ id: profile.id, name: event.target.value })
                      }
                      onKeyDown={(event) =>
                        event.key === "Escape" && setEditing(null)
                      }
                      className="min-w-0 flex-1 rounded border border-slate-700 bg-slate-900 px-2 py-1 text-sm outline-none focus:border-violet-400"
                    />
                    <button
                      type="submit"
                      className="cursor-pointer rounded bg-violet-500 px-2 py-1 text-sm text-white hover:bg-violet-400"
                    >
                      Save
                    </button>
                    <button
                      type="button"
                      className="cursor-pointer rounded px-2 py-1 text-sm text-slate-300 hover:bg-slate-800"
                      onClick={() => setEditing(null)}
                    >
                      Cancel
                    </button>
                  </form>
                ) : (
                  <>
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm text-slate-100">
                        {profile.name}
                        {profile.active && (
                          <span className="ml-2 rounded bg-violet-500/20 px-1.5 py-0.5 text-[11px] text-violet-200">
                            Active
                          </span>
                        )}
                        {profile.isDefault && (
                          <span className="ml-2 text-xs text-slate-500">
                            Default
                          </span>
                        )}
                      </p>
                      <p className="truncate text-xs text-slate-500">
                        {profile.githubLogin
                          ? `GitHub @${profile.githubLogin}`
                          : "No GitHub account yet"}
                      </p>
                    </div>
                    <button
                      type="button"
                      className="cursor-pointer rounded px-2 py-1 text-sm text-slate-300 hover:bg-slate-800"
                      onClick={() =>
                        setEditing({ id: profile.id, name: profile.name })
                      }
                    >
                      Rename
                    </button>
                    {blocker ? (
                      <Tooltip label={blocker} side="top" align="end">
                        <span
                          tabIndex={0}
                          className="rounded px-2 py-1 text-sm text-slate-600"
                        >
                          Delete
                        </span>
                      </Tooltip>
                    ) : (
                      <button
                        type="button"
                        className="cursor-pointer rounded px-2 py-1 text-sm text-rose-300 hover:bg-rose-500/10 disabled:cursor-not-allowed disabled:text-slate-600 disabled:hover:bg-transparent"
                        onClick={() => setConfirming(profile.id)}
                      >
                        Delete
                      </button>
                    )}
                  </>
                )}
              </div>
              {confirming === profile.id && (
                <div
                  role="alertdialog"
                  aria-label={`Delete ${profile.name}`}
                  className="mt-2 rounded border border-rose-500/40 bg-rose-500/10 p-3 text-sm"
                >
                  <p className="text-rose-100">
                    Delete {profile.name}? Its accounts, projects, templates and
                    activity are removed, and its secrets are removed from the
                    keychain. This can't be undone.
                  </p>
                  <div className="mt-2 flex gap-2">
                    <button
                      type="button"
                      className="cursor-pointer rounded bg-rose-500 px-3 py-1 text-white hover:bg-rose-400"
                      onClick={() =>
                        void run(() => api.deleteProfile?.(profile.id)).then(
                          () => setConfirming(null),
                        )
                      }
                    >
                      Delete profile
                    </button>
                    <button
                      type="button"
                      className="cursor-pointer rounded px-3 py-1 text-slate-300 hover:bg-slate-800"
                      onClick={() => setConfirming(null)}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}
            </li>
          );
        })}
      </ul>
      {error && <p className="text-sm text-rose-300">{error}</p>}
    </section>
  );
}
