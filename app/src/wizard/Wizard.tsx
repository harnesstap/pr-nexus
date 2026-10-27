import { useEffect, useMemo, useState } from "react";
import type { Api, Bootstrap, MappingFile, OnboardingState } from "../api";
import { errorMessage } from "../api";
import { MappingFileButton } from "../mapping/MappingFileButton";
import { OrgOption } from "../orgs";
import { DEFAULT_REDIRECT, SlackPanel, type SlackFields } from "../slack/SlackPanel";
import { WorkspaceSelect } from "../slack/WorkspaceSelect";
import { chosenWorkspace, csvHasWorkspaceColumn, rememberWorkspace } from "../slack/workspaces";
import { backEnabled, nextEnabled, nextHint, nextLabel, showSkip, stepTitle, type StepModel } from "./logic";
import { plainText } from "../textInput";
import { CheckIcon, ChevronLeftIcon, ChevronRightIcon, CopyIcon, GitHubIcon, LogoIcon, RefreshIcon, SkipIcon } from "../icons";

const ARM_DELAY_MS = 800;

const CSV_HEADER = "github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id";
const CSV_EXAMPLE = `${CSV_HEADER}
acme,platform,Platform,#platform-reviews,C0123ABCD
acme,web-app,Web App,#web-reviews,C0456EFGH`;

interface Props {
  api: Api;
  boot: Bootstrap;
  onFinished: () => void;
  /** Set when re-running setup from Settings: shows Back to PR Nexus, and keeps setup marked finished meanwhile. */
  onCancel?: () => void;
}

export function Wizard({ api, boot, onFinished, onCancel }: Props) {
  const steps = boot.steps;
  // A re-run must not drop the user back into setup on the next launch if they quit halfway.
  const finishedAt = onCancel ? boot.onboarding.finished_at : null;
  const start = Math.max(0, steps.indexOf(boot.resume_step));
  const [index, setIndex] = useState(start);
  const [completed, setCompleted] = useState<string[]>(boot.onboarding.completed);
  const [slackSkipped, setSlackSkipped] = useState(boot.onboarding.slack_skipped);
  const [mappingSkipped, setMappingSkipped] = useState(boot.onboarding.mapping_skipped);
  const [githubConnected, setGithubConnected] = useState(boot.github_connected);
  const [login, setLogin] = useState(boot.github_login);
  const [orgs, setOrgs] = useState<string[]>(boot.tracked_orgs);
  const [choices, setChoices] = useState<string[]>(boot.org_choices);
  const [ssoOrgs, setSsoOrgs] = useState<string[]>(boot.orgs_needing_sso);
  const [personal, setPersonal] = useState(boot.include_personal);
  const [orgFilter, setOrgFilter] = useState("");
  const [gh, setGh] = useState({ ready: boot.gh_ready, detail: boot.gh_detail, probed: boot.gh_probed });
  // Until the user picks one, the mode follows the gh probe.
  const [chosenMode, setChosenMode] = useState<"gh_cli" | "pat" | "device" | null>(null);
  const [pat, setPat] = useState("");
  const [deviceCode, setDeviceCode] = useState<string | null>(null);
  const [slackFields, setSlackFields] = useState<SlackFields>({
    clientId: boot.slack_client_id,
    clientSecret: "",
    redirectUri: boot.slack_redirect_uri || DEFAULT_REDIRECT,
  });
  const [workspaces, setWorkspaces] = useState(boot.slack_workspaces);
  const [csv, setCsv] = useState("");
  const [csvFile, setCsvFile] = useState<MappingFile | null>(null);
  const [pickedWorkspace, setPickedWorkspace] = useState("");
  const workspace = chosenWorkspace(workspaces, pickedWorkspace);
  const [mappingNote, setMappingNote] = useState(
    boot.mapping_count > 0 ? `${boot.mapping_count} mapping rows` : "",
  );
  const [syncMessage, setSyncMessage] = useState(boot.sync.message);
  const [syncSettled, setSyncSettled] = useState(false);
  const [syncFailed, setSyncFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function chooseMode(next: "gh_cli" | "pat" | "device") {
    setChosenMode(next);
  }

  // After setup, bootstrap skips the gh probe (a subprocess); Re-run setup asks here.
  useEffect(() => {
    if (gh.probed) {
      return;
    }
    let cancel = false;
    void api
      .probeGh()
      .then((status) => {
        if (!cancel) {
          setGh({ ready: status.gh_ready, detail: status.gh_detail, probed: true });
        }
      })
      .catch(() => !cancel && setGh((current) => ({ ...current, probed: true })));
    return () => {
      cancel = true;
    };
  }, [api, gh.probed]);

  const mode = chosenMode ?? (gh.ready ? "gh_cli" : "pat");

  const step = steps[index] ?? "choose_vcs";
  // The Done screen's button sits where Skip/Next were; a short delay keeps a
  // double click on the previous step from skipping the summary.
  const [armedIndex, setArmedIndex] = useState<number | null>(null);
  const armed = armedIndex === index;
  useEffect(() => {
    if (step !== "done") {
      return;
    }
    const timer = window.setTimeout(() => setArmedIndex(index), ARM_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [step, index]);

  // Bumped by "Retry sync" to run the first sync again.
  const [syncRun, setSyncRun] = useState(0);
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
        setSyncFailed(status.phase === "error");
        setSyncSettled(true);
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setError(errorMessage(err));
          setSyncMessage("Sync failed");
          setSyncFailed(true);
          setSyncSettled(true);
        }
      });
    return () => {
      cancel = true;
    };
  }, [api, step, syncRun]);

  function retrySync() {
    setError(null);
    setSyncSettled(false);
    setSyncFailed(false);
    setSyncMessage("Syncing…");
    setSyncRun((run) => run + 1);
  }

  const model: StepModel = {
    vcs: "github",
    chat: "slack",
    githubConnected,
    orgCount: orgs.length,
    personal,
    syncSettled,
    csvFilled: csv.trim().length > 0,
    slackConnected: workspaces.some((item) => item.signed_in),
  };
  const hint = busy ? null : nextHint(step, model);
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
      finished_at: finishedAt,
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
        await api.saveSlack(slackFields.clientId, slackFields.clientSecret, slackFields.redirectUri);
      }
      if (step === "chat_mapping") {
        if (csv.trim()) {
          const summary = await api.importMapping(csv, workspace);
          rememberWorkspace(workspace);
          setMappingNote(`${summary.rows} rows imported`);
          setMappingSkipped(false);
        } else if (boot.mapping_count === 0) {
          setMappingSkipped(true);
        }
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
        finished_at: finishedAt,
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
        finished_at: finishedAt,
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
      setSsoOrgs(session.organizations_needing_sso);
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

  return (
    <main className="mx-auto flex min-h-screen max-w-3xl flex-col px-6 py-8 text-slate-100">
      <header>
        <div className="flex items-center justify-between">
          <p className="text-sm text-slate-400">PR Nexus setup</p>
          {onCancel && (
            <button type="button" className="flex items-center gap-1 rounded px-2 py-1 text-sm text-slate-300 hover:bg-slate-800" onClick={onCancel}>
              <ChevronLeftIcon />
              Back to PR Nexus
            </button>
          )}
        </div>
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
            name="GitHub"
            detail="github.com"
          />
        )}
        {step === "choose_chat" && (
          <ProviderStep
            prompt="Where do you ask for reviews?"
            name="Slack"
            detail="One or more workspaces"
          />
        )}
        {step === "connect_vcs" && (
          <div className="space-y-4">
            <label className="flex items-start gap-3">
              <input
                type="radio"
                name="github-auth"
                checked={mode === "gh_cli"}
                onChange={() => chooseMode("gh_cli")}
              />
              <span>
                Use the gh CLI token
                <span className="mt-1 block text-sm text-slate-400">{gh.detail}</span>
              </span>
            </label>
            <label className="flex items-start gap-3">
              <input
                type="radio"
                name="github-auth"
                checked={mode === "pat"}
                onChange={() => chooseMode("pat")}
              />
              <span className="flex-1">
                Paste a personal access token
                <input {...plainText}
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
                onChange={() => chooseMode("device")}
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
                className="flex items-center gap-2 rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
                onClick={() => void connect()}
                disabled={busy || (mode === "device" && !boot.oauth_client_configured)}
              >
                {mode === "device" ? <CopyIcon /> : <GitHubIcon />}
                {mode === "device" ? "Copy & open" : "Connect"}
              </button>
              {mode === "device" && deviceCode && (
                <button type="button" className="flex items-center gap-2 rounded border border-slate-600 px-3 py-2 text-sm" onClick={() => void poll()}>
                  <CheckIcon />
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
            <p className="text-slate-300">Pick the GitHub orgs whose PRs PR Nexus should show.</p>
            <input {...plainText}
              className="mt-4 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2"
              placeholder="Filter orgs…"
              value={orgFilter}
              onChange={(event) => setOrgFilter(event.target.value)}
            />
            <ul className="mt-4 space-y-2">
              {filteredChoices.map((org) => (
                <li key={org}>
                  <OrgOption
                    org={org}
                    checked={orgs.includes(org)}
                    needsSso={ssoOrgs.includes(org)}
                    onToggle={() =>
                      setOrgs((current) =>
                        current.includes(org) ? current.filter((item) => item !== org) : [...current, org],
                      )
                    }
                    onOpen={(url) => void api.openExternal(url)}
                  />
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
          <SlackPanel
            api={api}
            fields={slackFields}
            onFieldsChange={setSlackFields}
            secretSaved={boot.slack_secret_saved}
            workspaces={workspaces}
            onWorkspacesChange={setWorkspaces}
          />
        )}
        {step === "chat_mapping" && (
          <div className="space-y-3">
            <p className="text-slate-300">
              Tell PR Nexus which Slack channel each GitHub team reviews in. When a PR requests a team review, PR Nexus
              suggests posting the request in that team's channel. This step is optional.
            </p>
            <div className="text-sm">
              <span>Default Slack workspace</span>
              <span className="mt-0.5 block text-xs text-slate-400">
                Used for rows without a <code>slack_workspace</code> column.
              </span>
              <div className="mt-1">
                <WorkspaceSelect
                  label="Default Slack workspace"
                  className="w-full rounded border border-slate-700 bg-slate-900 px-3 py-2 disabled:opacity-60"
                  workspaces={workspaces}
                  value={workspace}
                  onChange={setPickedWorkspace}
                  onConnect={() => {
                    const target = steps.indexOf("connect_chat");
                    if (target >= 0) {
                      setError(null);
                      setIndex(target);
                      persist(target).catch((err: unknown) => setError(errorMessage(err)));
                    }
                  }}
                  connectLabel="Back to Connect Slack"
                />
              </div>
              {csv.trim() !== "" && !workspace && !csvHasWorkspaceColumn(csv) && (
                <p className="mt-1 text-xs text-amber-200">
                  Connect a Slack workspace, or add a <code>slack_workspace</code> column to the CSV.
                </p>
              )}
            </div>
            <label className="block text-sm">
              Mapping CSV
              <span className="mt-0.5 block text-xs text-slate-400">
                One row per GitHub team. Columns: org, team slug, team display name, Slack channel name, Slack channel id
                (starts with C or G; copy it from the channel's details in Slack). An optional <code>slack_workspace</code>{" "}
                column overrides the default workspace.
              </span>
              <textarea {...plainText}
                className="mt-1 h-36 w-full rounded border border-slate-700 bg-slate-900 px-3 py-2 font-mono text-xs"
                placeholder={CSV_EXAMPLE}
                value={csv}
                onChange={(event) => {
                  setCsv(event.target.value);
                  setCsvFile(null);
                }}
              />
            </label>
            <MappingFileButton
              api={api}
              file={csvFile}
              disabled={busy}
              onLoaded={(file) => {
                setError(null);
                setCsv(file.csv);
                setCsvFile(file);
              }}
              onError={setError}
            />
            <details className="text-xs text-slate-400">
              <summary className="cursor-pointer">Example (made-up data)</summary>
              <pre className="mt-2 overflow-x-auto rounded bg-slate-900 p-2 font-mono">{CSV_EXAMPLE}</pre>
            </details>
            {mappingNote && <p className="text-sm text-slate-300">{mappingNote}</p>}
            <p className="text-sm text-slate-400">
              {csv.trim()
                ? "Import & next imports these rows and continues."
                : "Choose a CSV file or paste one, then press Import & next, or press Next / Skip to continue without one. You can map channels later in Settings."}
            </p>
          </div>
        )}
        {step === "done" && (
          <div className="space-y-3">
            <p className="text-slate-300">You're all set. Here's what PR Nexus will use:</p>
            <p>GitHub: {login ? `@${login}` : "not connected"} · {orgs.length} orgs · personal repos {personal ? "on" : "off"}</p>
            <p>
              Slack:{" "}
              {workspaces.some((item) => item.signed_in)
                ? workspaces
                    .filter((item) => item.signed_in)
                    .map((item) => item.team_name)
                    .join(", ")
                : slackSkipped
                  ? "skipped"
                  : "not connected"}{" "}
              · mapping:{" "}
              {mappingSkipped ? "skipped" : mappingNote || "imported"}
            </p>
            <p className="text-slate-300" aria-live="polite">
              First sync: {syncMessage}
            </p>
            {syncFailed && (
              <div className="space-y-2">
                <p className="text-sm text-slate-400">
                  You can retry now, or open PR Nexus and use the refresh button in the sidebar later.
                </p>
                <button
                  type="button"
                  className="flex items-center gap-2 rounded border border-slate-600 px-3 py-1.5 text-sm disabled:opacity-40"
                  onClick={retrySync}
                  disabled={busy || !syncSettled}
                >
                  <RefreshIcon />
                  Retry sync
                </button>
              </div>
            )}
          </div>
        )}
        {error && <p className="mt-4 text-sm text-rose-300">{error}</p>}
      </section>
      {hint && (
        <p className="mt-4 text-right text-sm text-slate-400" role="status">
          {hint}
        </p>
      )}
      <footer className="mt-4 flex justify-end gap-2 border-t border-slate-800 pt-4">
        <button
          type="button"
          className="flex items-center gap-1 rounded px-3 py-2 text-sm disabled:opacity-40"
          onClick={() => void goBack()}
          disabled={!backEnabled(index)}
        >
          <ChevronLeftIcon />
          Back
        </button>
        {showSkip(step) && (
          <button type="button" className="flex items-center gap-1.5 rounded px-3 py-2 text-sm" onClick={() => void skip()}>
            <SkipIcon />
            Skip
          </button>
        )}
        {step === "done" ? (
          <button
            type="button"
            className="flex items-center gap-2 rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
            onClick={() => void openApp()}
            disabled={busy || !syncSettled || !armed}
          >
            <LogoIcon />
            Open PR Nexus
          </button>
        ) : (
          <button
            type="button"
            className="flex items-center gap-1 rounded bg-violet-500 px-3 py-2 text-sm font-medium text-white disabled:opacity-40"
            onClick={() => void goNext()}
            disabled={!canNext}
          >
            {nextLabel(step, model)}
            <ChevronRightIcon />
          </button>
        )}
      </footer>
    </main>
  );
}

/** v1 supports one provider per step, shown as a single preselected card. */
function ProviderStep({ prompt, name, detail }: { prompt: string; name: string; detail: string }) {
  return (
    <div>
      <p className="mb-4 text-slate-300">{prompt}</p>
      <div role="radiogroup" aria-label={prompt} className="grid gap-3 sm:grid-cols-3">
        <button type="button" role="radio" aria-checked="true" className="rounded-lg border border-violet-400 bg-violet-500/10 px-4 py-5 text-left">
          <span className="block font-medium">{name}</span>
          <span className="mt-1 block text-sm text-slate-400">{detail}</span>
        </button>
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
