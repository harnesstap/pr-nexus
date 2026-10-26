import { invoke } from "@tauri-apps/api/core";

export type StepId =
  | "choose_vcs"
  | "choose_chat"
  | "connect_vcs"
  | "choose_vcs_scope"
  | "connect_chat"
  | "chat_mapping"
  | "done";

export interface OnboardingState {
  step: StepId;
  completed: string[];
  mapping_skipped: boolean;
  slack_skipped: boolean;
  finished_at: string | null;
}

export interface SyncStatus {
  phase: string;
  message: string;
  synced_at: string | null;
  paused: boolean;
}

export interface Bootstrap {
  onboarding: OnboardingState;
  resume_step: StepId;
  steps: StepId[];
  keyring_warning: string | null;
  keyring_available: boolean;
  github_login: string | null;
  github_connected: boolean;
  tracked_orgs: string[];
  include_personal: boolean;
  org_choices: string[];
  slack_client_id: string;
  slack_redirect_uri: string;
  mapping_count: number;
  sync: SyncStatus;
  env_token: boolean;
  oauth_client_configured: boolean;
  gh_ready: boolean;
  gh_detail: string;
}

export interface GithubSession {
  login: string;
  organizations: string[];
}

export interface DevicePrompt {
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export interface ListRow {
  id: number;
  number: number;
  title: string;
  url: string;
  body: string;
  owner: string;
  repo: string;
  author_login: string;
  is_draft: boolean;
  state: string;
  checks: string;
  review_decision: string | null;
  unresolved_threads: number;
  total_threads: number;
  involvement: number;
  updated_at: string;
  created_at: string;
  hidden: boolean;
  project_ids: number[];
  readiness: "ready" | "blocked" | "unknown";
  readiness_notes: string[];
  slack: {
    requested_at: string;
    last_bumped_at: string | null;
    bump_count: number;
    workspace: string;
    channel_id: string;
    channel_name: string;
    permalink: string | null;
    root_ts: string;
    source: string;
  } | null;
}

export interface ProjectDto {
  id: number;
  name: string;
  color: string;
  archived: boolean;
  description: string;
  sort_order: number;
}

export interface Api {
  bootstrap(): Promise<Bootstrap>;
  saveProviders(vcs: string, chat: string): Promise<void>;
  saveOnboarding(state: OnboardingState): Promise<void>;
  connectGithub(mode: "gh_cli" | "pat", pat?: string): Promise<GithubSession>;
  startDevice(): Promise<DevicePrompt>;
  pollDevice(): Promise<{ status: string; login: string | null }>;
  saveOrgs(orgs: string[], includePersonal: boolean): Promise<void>;
  saveSlack(clientId: string, clientSecret: string, redirectUri: string): Promise<void>;
  slackManifestUrl(redirectUri: string): Promise<string>;
  importMapping(csv: string, workspace: string): Promise<{ rows: number }>;
  finish(): Promise<SyncStatus>;
  syncNow(): Promise<SyncStatus>;
  syncStatus(): Promise<SyncStatus>;
  listRows(): Promise<ListRow[]>;
  listProjects(): Promise<ProjectDto[]>;
  renameProject(id: number, name: string): Promise<void>;
  archiveProject(id: number, archived: boolean): Promise<void>;
  deleteProject(id: number): Promise<void>;
  loadViewState(viewKey: string): Promise<string | null>;
  saveViewState(viewKey: string, stateJson: string): Promise<void>;
  openExternal(url: string): Promise<void>;
}

export const tauriApi: Api = {
  bootstrap: () => invoke("bootstrap"),
  saveProviders: (vcs, chat) => invoke("save_providers", { vcs, chat }),
  saveOnboarding: (state) => invoke("save_onboarding", { patch: state }),
  connectGithub: (mode, pat) => invoke("connect_github", { mode, pat }),
  startDevice: () => invoke("start_device_flow"),
  pollDevice: () => invoke("poll_device_flow"),
  saveOrgs: (orgs, includePersonal) =>
    invoke("save_orgs", { orgs, includePersonal }),
  saveSlack: (clientId, clientSecret, redirectUri) =>
    invoke("save_slack_settings", {
      clientId,
      clientSecret: clientSecret || null,
      redirectUri,
    }),
  slackManifestUrl: (redirectUri) => invoke("slack_manifest_url", { redirectUri }),
  importMapping: (csv, workspace) => invoke("import_mapping", { csv, workspace }),
  finish: () => invoke("finish_onboarding"),
  syncNow: () => invoke("sync_now"),
  syncStatus: () => invoke("sync_status"),
  listRows: () => invoke("list_rows"),
  listProjects: () => invoke("list_projects"),
  renameProject: (id, name) => invoke("rename_project", { id, name }),
  archiveProject: (id, archived) => invoke("archive_project", { id, archived }),
  deleteProject: (id) => invoke("delete_project", { id }),
  loadViewState: (viewKey) => invoke("load_view_state", { viewKey }),
  saveViewState: (viewKey, stateJson) => invoke("save_view_state", { viewKey, stateJson }),
  openExternal: (url) => invoke("open_external", { url }),
};

export function errorMessage(err: unknown): string {
  if (typeof err === "string") {
    return err;
  }
  if (err && typeof err === "object" && "message" in err) {
    const message = (err as { message?: unknown }).message;
    if (typeof message === "string") {
      const sso = (err as { sso_url?: unknown }).sso_url;
      return typeof sso === "string" ? `${message} ${sso}` : message;
    }
  }
  return "Something went wrong";
}
