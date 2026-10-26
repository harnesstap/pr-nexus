import { useEffect, useMemo, useState } from "react";
import type { Api, Bootstrap, OnboardingState } from "../api";
import { errorMessage } from "../api";
import { backEnabled, nextEnabled, showSkip, stepTitle, type StepModel } from "./logic";

const DEFAULT_REDIRECT = "http://127.0.0.1:47821/mcp/oauth/callback";

interface Props {
  api: Api;
  boot: Bootstrap;
  onFinished: () => void;
}

export function Wizard({ api, boot, onFinished }: Props) {
  const steps = boot.steps;
  const start = Math.max(0, steps.indexOf(boot.resume_step));
  const [index, setIndex] = useState(start);
  const [completed, setCompleted] = useState<string[]>(boot.onboarding.completed);
  const [slackSkipped, setSlackSkipped] = useState(boot.onboarding.slack_skipped);
  const [mappingSkipped, setMappingSkipped] = useState(boot.onboarding.mapping_skipped);
  const [githubConnected, setGithubConnected] = useState(boot.github_connected);
  const [login, setLogin] = useState(boot.github_login);
  const [orgs, setOrgs] = useState<string[]>(boot.tracked_orgs);
  const [choices, setChoices] = useState<string[]>(boot.org_choices);
  const [personal, setPersonal] = useState(boot.include_personal);
  const [orgFilter, setOrgFilter] = useState("");
  const [mode, setMode] = useState<"gh_cli" | "pat" | "device">(boot.gh_ready ? "gh_cli" : "pat");
  const [pat, setPat] = useState("");
  const [deviceCode, setDeviceCode] = useState<string | null>(null);
  const [clientId, setClientId] = useState(boot.slack_client_id);
  const [clientSecret, setClientSecret] = useState("");
  const [redirectUri, setRedirectUri] = useState(boot.slack_redirect_uri || DEFAULT_REDIRECT);
  const [csv, setCsv] = useState("");
  const [workspace, setWorkspace] = useState("");
  const [mappingNote, setMappingNote] = useState(
    boot.mapping_count > 0 ? `${boot.mapping_count} mapping rows` : "",
  );
  const [syncMessage, setSyncMessage] = useState(boot.sync.message);
  const [syncSettled, setSyncSettled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const step = steps[index] ?? "choose_vcs";

  useEffect(() => {
    if (step !== "done") {
      return;
    }
    let cancel = false;
    void api
      .syncNow()
      .then((status) => {
        if (cancel) {
          return;
        }
        setSyncMessage(status.message);
        setSyncSettled(true);
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setError(errorMessage(err));
          setSyncMessage("Sync failed");
          setSyncSettled(true);
        }
      });
    return () => {
      cancel = true;
    };
  }, [api, step]);
  const model: StepModel = {
    vcs: "github",
    chat: "slack",
    githubConnected,
    orgCount: orgs.length,
    personal,
    syncSettled,
  };
  const canNext = nextEnabled(step, model) && !busy;
  const filteredChoices = useMemo(
    () => choices.filter((org) => org.toLowerCase().includes(orgFilter.trim().toLowerCase())),
    [choices, orgFilter],
  );

  async function persist(nextIndex: number, extra?: Partial<OnboardingState>) {
    const nextStep = steps[nextIndex] ?? step;
    const state: OnboardingState = {
      step: nextStep,
      completed,
      mapping_skipped: extra?.mapping_skipped ?? mappingSkipped,
      slack_skipped: extra?.slack_skipped ?? slackSkipped,
      finished_at: null,
    };
    await api.saveOnboarding(state);
  }

  async function goNext() {
    setError(null);
    setBusy(true);
    try {
      if (step === "choose_vcs" || step === "choose_chat") {
        await api.saveProviders("github", "slack");
      }
      if (step === "connect_vcs" && !githubConnected) {
        return;
      }
      if (step === "choose_vcs_scope") {
        await api.saveOrgs(orgs, personal);
      }
      if (step === "connect_chat") {
        await api.saveSlack(clientId, clientSecret, redirectUri);
      }
      const done = [...completed];
      if (!done.includes(step)) {
        done.push(step);
      }
      setCompleted(done);
      const next = Math.min(index + 1, steps.length - 1);
      await api.saveOnboarding({
        step: steps[next] ?? step,
        completed: done,
        mapping_skipped: mappingSkipped,
        slack_skipped: slackSkipped,
        finished_at: null,
      });
      setIndex(next);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function goBack() {
    if (!backEnabled(index)) {
      return;
    }
    const next = index - 1;
    setIndex(next);
    try {
      await persist(next);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function skip() {
    setError(null);
    if (step === "connect_chat") {
      setSlackSkipped(true);
    }
    if (step === "chat_mapping") {
      setMappingSkipped(true);
    }
    const done = completed.includes(step) ? completed : [...completed, step];
    setCompleted(done);
    const next = Math.min(index + 1, steps.length - 1);
    try {
      await api.saveOnboarding({
        step: steps[next] ?? step,
        completed: done,
        mapping_skipped: step === "chat_mapping" ? true : mappingSkipped,
        slack_skipped: step === "connect_chat" ? true : slackSkipped,
        finished_at: null,
      });
      setIndex(next);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function connect() {
    setError(null);
    setBusy(true);
    try {
      if (mode === "device") {
        const prompt = await api.startDevice();
        setDeviceCode(prompt.user_code);
        await api.openExternal(prompt.verification_uri);
        return;
      }
      const session = await api.connectGithub(mode === "pat" ? "pat" : "gh_cli", pat);
      setGithubConnected(true);
      setLogin(session.login);
      setChoices((current) => mergeOrgs(current, session.organizations));
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function poll() {
    setBusy(true);
    setError(null);
    try {
      const result = await api.pollDevice();
      if (result.status === "connected" && result.login) {
        setGithubConnected(true);
        setLogin(result.login);
        setDeviceCode(null);
      }
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function importCsv() {
    setBusy(true);
    setError(null);
    try {
      const summary = await api.importMapping(csv, workspace);
      setMappingNote(`${summary.rows} rows imported`);
      setMappingSkipped(false);
      const done = completed.includes(step) ? completed : [...completed, step];
      setCompleted(done);
      const next = Math.min(index + 1, steps.length - 1);
      await api.saveOnboarding({
        step: steps[next] ?? step,
        completed: done,
        mapping_skipped: false,
        slack_skipped: slackSkipped,
        finished_at: null,
      });
      setIndex(next);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function openApp() {
    setBusy(true);
    setError(null);
    try {
      const status = await api.finish();
      setSyncMessage(status.message);
      setSyncSettled(true);
      onFinished();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function openManifest() {
    setError(null);
    try {
      const url = await api.slackManifestUrl(redirectUri);
      await api.openExternal(url);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  return (
    <main className="mx-auto flex min-h-screen max-w-3xl flex-col px-6 py-8 text-slate-100">
      <header>
        <p className="text-sm text-slate-400">PR-Nexus setup</p>
        <div className="mt-3 flex items-center gap-2" aria-hidden="true">
          {steps.map((item, dot) => (
            <span
              key={item}
              className={`h-2.5 w-2.5 rounded-full ${dot <= index ? "bg-violet-400" : "bg-slate-700"}`}
            />
          ))}
        </div>
        <h1 className="mt-4 text-xl font-semibold">
          Step {index + 1} of {steps.length} · {stepTitle(step)}
        </h1>
      </header>
      <section className="mt-8 flex-1">
        {step === "choose_vcs" && (
          <ProviderStep
            prompt="Where do your pull requests live?"
            selected="GitHub"
            cards={[
              ["GitHub", "github.com", false],
              ["GitLab", "Coming soon", true],
              ["Gitea", "Coming soon", true],
            ]}
          />
        )}
        {step === "choose_chat" && (
          <ProviderStep
            prompt="Where do you ask for reviews?"
            selected="Slack"
            cards={[
              ["Slack", "N workspaces", false],
              ["Discord", "Coming soon", true],
              ["Mattermost", "Coming soon", true],
            ]}
          />
        )}
        {step === "connect_vcs" && (
          <div className="space-y-4">
            <label className="flex items-start gap-3">
              <input
                type="radio"
                name="github-auth"
                checked={mode === "gh_cli"}
                onChange={() => setMode("gh_cli")}
              />
              <span>
                Use the gh CLI token
                <span className="mt-1 block text-sm text-slate-400">{boot.gh_detail}</span>
              </span>
            </label>
            <label className="flex items-start gap-3">
              <input
                type="radio"
                name="github-auth"
                checked={mode === "pat"}
                onChange={() => setMode("pat")}
              />
              <span className="flex-1">
                Paste a personal access token
                <input
                  className="mt-2 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2"
                  type="password"
                  autoComplete="off"
                  placeholder="github_pat_…"
                  value={pat}
                  onChange={(event) => setPat(event.target.value)}
                />
              </span>
            </label>
            <label className="flex items-start gap-3">
              <input
                type="radio"
                name="github-auth"
                checked={mode === "device"}
                onChange={() => setMode("device")}
              />
              <span>
                Sign in with GitHub (device flow)
                {deviceCode && (
                  <span className="mt-1 block font-mono text-sm">code: {deviceCode}</span>
                )}
                {!boot.oauth_client_configured && (
                  <span className="mt-1 block text-sm text-slate-400">
                    Needs a client id in github.oauth_client_id or PR_NEXUS_GITHUB_OAUTH_CLIENT_ID.
                  </span>
                )}
              </span>
            </label>
            <div className="flex gap-2">
              <button
                type="button"
                className="rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
                onClick={() => void connect()}
                disabled={busy || (mode === "device" && !boot.oauth_client_configured)}
              >
                {mode === "device" ? "Copy & open" : "Connect"}
              </button>
              {mode === "device" && deviceCode && (
                <button type="button" className="rounded border border-slate-600 px-3 py-2 text-sm" onClick={() => void poll()}>
                  I've authorized
                </button>
              )}
            </div>
            {githubConnected && (
              <p className="text-sm text-emerald-300">Connected as @{login}</p>
            )}
          </div>
        )}
        {step === "choose_vcs_scope" && (
          <div>
            <p className="text-slate-300">Pick the GitHub orgs whose PRs PR-Nexus should show.</p>
            <input
              className="mt-4 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2"
              placeholder="Filter orgs…"
              value={orgFilter}
              onChange={(event) => setOrgFilter(event.target.value)}
            />
            <ul className="mt-4 space-y-2">
              {filteredChoices.map((org) => (
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
              {filteredChoices.length === 0 && (
                <li className="text-sm text-slate-400">No orgs yet. Connect GitHub, then sync.</li>
              )}
            </ul>
            <label className="mt-4 flex items-center gap-2">
              <input
                type="checkbox"
                checked={personal}
                onChange={(event) => setPersonal(event.target.checked)}
              />
              Include my personal repos{login ? ` (@${login}/*)` : ""}
            </label>
          </div>
        )}
        {step === "connect_chat" && (
          <div className="space-y-4">
            <p className="rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-100">
              Slack sign-in comes in M10. You can save these settings and continue, or skip.
            </p>
            <button type="button" className="text-sm text-violet-300 underline" onClick={() => void openManifest()}>
              Create your Slack app
            </button>
            <p className="text-sm text-slate-400">
              The link opens Slack with the PR-Nexus manifest (name, user scopes, redirect URI, MCP on).
              No client id ships with the app.
            </p>
            <label className="block text-sm">
              Client ID
              <input
                className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2"
                value={clientId}
                onChange={(event) => setClientId(event.target.value)}
              />
            </label>
            <label className="block text-sm">
              Client secret
              <input
                className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2"
                type="password"
                autoComplete="off"
                placeholder="optional · keychain"
                value={clientSecret}
                onChange={(event) => setClientSecret(event.target.value)}
              />
            </label>
            <label className="block text-sm">
              Redirect URI
              <input
                className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2"
                value={redirectUri}
                onChange={(event) => setRedirectUri(event.target.value)}
              />
            </label>
            <button type="button" className="rounded border border-slate-600 px-3 py-2 text-sm opacity-50" disabled>
              Add another workspace
            </button>
          </div>
        )}
        {step === "chat_mapping" && (
          <div className="space-y-3">
            <p>Tell PR-Nexus which Slack channel each GitHub team reviews in.</p>
            <label className="block text-sm">
              Target workspace for rows without one
              <input
                className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2"
                value={workspace}
                onChange={(event) => setWorkspace(event.target.value)}
                placeholder="workspace domain"
              />
            </label>
            <textarea
              className="h-40 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2 font-mono text-xs"
              placeholder="github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id"
              value={csv}
              onChange={(event) => setCsv(event.target.value)}
            />
            <button type="button" className="rounded bg-slate-800 px-3 py-2 text-sm" onClick={() => void importCsv()}>
              Import & next
            </button>
            {mappingNote && <p className="text-sm text-slate-300">{mappingNote}</p>}
            <p className="text-sm text-slate-400">
              You can also map channels later in Settings. Skip leaves the mapping empty.
            </p>
          </div>
        )}
        {step === "done" && (
          <div className="space-y-2">
            <p>GitHub: {login ? `@${login}` : "not connected"} · {orgs.length} orgs · personal repos {personal ? "on" : "off"}</p>
            <p>
              Slack: {slackSkipped ? "skipped" : "settings saved"} · mapping:{" "}
              {mappingSkipped ? "skipped" : mappingNote || "imported"}
            </p>
            <p className="text-slate-300">{syncMessage}</p>
          </div>
        )}
        {error && <p className="mt-4 text-sm text-rose-300">{error}</p>}
      </section>
      <footer className="mt-8 flex justify-end gap-2 border-t border-slate-800 pt-4">
        <button
          type="button"
          className="rounded px-3 py-2 text-sm disabled:opacity-40"
          onClick={() => void goBack()}
          disabled={!backEnabled(index)}
        >
          Back
        </button>
        {showSkip(step) && (
          <button type="button" className="rounded px-3 py-2 text-sm" onClick={() => void skip()}>
            Skip
          </button>
        )}
        {step === "done" ? (
          <button
            type="button"
            className="rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
            onClick={() => void openApp()}
            disabled={busy || !syncSettled}
          >
            Open PR-Nexus
          </button>
        ) : (
          <button
            type="button"
            className="rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
            onClick={() => void goNext()}
            disabled={!canNext}
          >
            Next
          </button>
        )}
      </footer>
    </main>
  );
}

function ProviderStep({
  prompt,
  selected,
  cards,
}: {
  prompt: string;
  selected: string;
  cards: [string, string, boolean][];
}) {
  return (
    <div>
      <p className="mb-4 text-slate-300">{prompt}</p>
      <div className="grid gap-3 sm:grid-cols-3">
        {cards.map(([name, detail, disabled]) => (
          <button
            key={name}
            type="button"
            role="radio"
            aria-checked={name === selected}
            disabled={disabled}
            className={`rounded-lg border px-4 py-5 text-left ${
              name === selected ? "border-violet-400 bg-violet-500/10" : "border-slate-700"
            } disabled:cursor-not-allowed disabled:opacity-50`}
          >
            <span className="block font-medium">{name}</span>
            <span className="mt-1 block text-sm text-slate-400">{detail}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function mergeOrgs(current: string[], extra: string[]): string[] {
  const next = [...current];
  for (const org of extra) {
    if (!next.some((item) => item.toLowerCase() === org.toLowerCase())) {
      next.push(org);
    }
  }
  return next;
}
