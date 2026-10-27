import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

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
  orgs_needing_sso: string[];
  slack_client_id: string;
  slack_redirect_uri: string;
  slack_secret_saved: boolean;
  slack_workspaces: SlackWorkspace[];
  /** Mentions search in the PR detail's Slack panel (§7.10). */
  slack_mentions: MentionsScope;
  /** Bumps are also sent to the channel (Slack's reply_broadcast). */
  bump_broadcast: boolean;
  mapping_count: number;
  sync: SyncStatus;
  env_token: boolean;
  oauth_client_configured: boolean;
  gh_ready: boolean;
  gh_detail: string;
}

/** Settings → Review and Sync (§7.12). Intervals are seconds. */
export interface Preferences {
  review_requester: string;
  min_bump_interval_hours: number;
  bulk_bump_max_threads: number;
  bump_broadcast: boolean;
  blocked_review_required_is_ready: boolean;
  ticket_key_regex: string;
  focused_interval_s: number;
  unfocused_interval_s: number;
  keep_closed_days: number;
}

export interface MappingRow {
  github_org: string;
  team_slug: string;
  team_name: string;
  slack_workspace: string;
  slack_channel_name: string;
  slack_channel_id: string;
}

export interface MappingEntry {
  row: MappingRow;
  /** `user`: edited in Settings or merged from a CSV; wins over `imported`. */
  source: "user" | "imported";
  disabled: boolean;
  has_imported: boolean;
}

export type ImportMode = "merge" | "replace";

/** A mapping CSV picked in the native file dialog and read by the backend. */
export interface MappingFile {
  path: string;
  name: string;
  csv: string;
}

export type ExportMode = "effective" | "user" | "imported";

export interface ImportPreview {
  added: MappingRow[];
  changed: { row: MappingRow; changes: { field: string; before: string; after: string }[] }[];
  unchanged: number;
  user_shadowed: MappingRow[];
}

export interface AppInfo {
  version: string;
  repository: string;
  db_path: string;
  log_dir: string;
  keyring_in_memory: boolean;
  os: string;
}

export interface UpdateInfo {
  version: string;
  url: string;
  command: string;
}

/** `off` (default), `public` channels, or `all` (also private channels and DMs). */
export type MentionsScope = "off" | "public" | "all";

/** One message of a known thread, parent first (§7.10). */
export interface ContextMessage {
  ts: string;
  at: string | null;
  author: string;
  user_id: string | null;
  text: string;
  permalink: string | null;
  is_bot: boolean;
}

/** A thread PR-Nexus posted for this PR, with what Slack returned. */
export interface ContextThread {
  workspace: string;
  channel_id: string;
  channel_name: string;
  root_ts: string;
  permalink: string | null;
  status: "active" | "closed" | "posted";
  close_reason: string | null;
  requested_at: string | null;
  /** Stored with the request; shown even when Slack cannot be read. */
  last_bumped_at: string | null;
  bump_count: number;
  messages: ContextMessage[];
  has_more: boolean;
  fetched_at: string | null;
  from_cache: boolean;
  /** Could not read now; `messages` may still hold the cached copy. */
  error: string | null;
}

export interface SearchHit {
  channel_id: string;
  channel_name: string;
  ts: string;
  author: string | null;
  text: string;
  permalink: string | null;
  is_bot: boolean;
}

export interface MentionGroup {
  workspace: string;
  hits: SearchHit[];
  error: string | null;
}

/** A Slack workspace signed in through the Slack MCP server (§7.11). */
export interface SlackWorkspace {
  id: number;
  team_name: string;
  /** `acme` for acme.slack.com. */
  domain: string;
  user_name: string;
  /** The grant is saved and Slack has not rejected it. */
  signed_in: boolean;
  /** Was connected, but the token is gone or rejected: offer Reconnect. */
  needs_sign_in: boolean;
}

export interface ReviewTarget {
  workspace: string;
  channel_id: string;
  channel_name: string;
}

export interface TargetChoice {
  target: ReviewTarget;
  teams: string[];
  source: "request" | "mapping";
}

export interface ReviewPreview {
  target: ReviewTarget;
  action: "request" | "bump";
  text: string;
  warnings: string[];
  signed_in: boolean;
  bump_count: number;
  permalink: string | null;
  /** A bump is also sent to the channel. */
  broadcast: boolean;
}

export type SlackIndicator = NonNullable<ListRow["slack"]>;

export interface ChannelKey {
  workspace: string;
  channel_id: string;
}

/** Bulk request/bump (§7.9). `target` sends everything to one channel. */
export interface BulkRequest {
  pr_ids: number[];
  target: ReviewTarget | null;
  single_top_level: ChannelKey[];
}

export interface BulkPr {
  pr_id: number;
  number: number;
  title: string;
  repo: string;
}

export interface BulkMessage {
  action: "request" | "bump" | "remind";
  prs: BulkPr[];
  text: string;
  permalink: string | null;
}

export interface BulkCard {
  target: ReviewTarget;
  messages: BulkMessage[];
  warnings: string[];
  signed_in: boolean;
  single_top_level: boolean;
  can_switch: boolean;
  fingerprint: string;
}

export interface BulkPreview {
  cards: BulkCard[];
  skipped: { pr: BulkPr; detail: string }[];
}

export interface BulkOutcome {
  pr: BulkPr;
  target: ReviewTarget | null;
  outcome: "sent" | "skipped" | "failed";
  detail: string;
  permalink: string | null;
}

export interface BulkResult {
  batch_id: string;
  results: BulkOutcome[];
  indicators: { pr_id: number; indicator: SlackIndicator | null }[];
}

export interface GithubSession {
  login: string;
  organizations: string[];
  organizations_needing_sso: string[];
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

export type RuleKind =
  | "ticket_key_in_branch"
  | "branch_regex"
  | "label"
  | "repo"
  | "title_regex";

export interface ProjectRuleDto {
  id: number | null;
  project_id: number;
  kind: RuleKind;
  pattern: string;
  priority: number;
}

export interface ProjectLinkDto {
  pr_id: number;
  source: "manual" | "rule" | "excluded";
  rule_id: number | null;
}

export interface TeamGap {
  team: string;
  reasons: string[];
}

export interface FileOwners {
  path: string;
  teams: string[];
}

/** Readiness, owners, and why for one PR (§6, §7.2). */
export interface PrDetail {
  id: number;
  readiness: "ready" | "blocked" | "unknown";
  readiness_notes: string[];
  head_sha: string;
  base_ref: string;
  codeowners_path: string | null;
  codeowners_error: string | null;
  codeowners_required: "true" | "false" | "unknown";
  codeowners_required_reason: string;
  code_owner_review_pending: boolean | null;
  mandatory_teams: string[];
  optional_teams: string[];
  approved_teams: string[];
  candidate_teams: string[];
  attribution_unknown: TeamGap[];
  requested_teams: string[];
  users_ignored: string[];
  rationale: string[];
  files: FileOwners[];
  files_truncated: boolean;
  rules_error: string | null;
  protection: string;
  from_cache: boolean;
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
  slackWorkspaces(): Promise<SlackWorkspace[]>;
  /** Opens the system browser and resolves after the redirect, a timeout, or cancel. */
  slackConnect(): Promise<SlackWorkspace>;
  slackCancelConnect(): Promise<void>;
  slackRemoveWorkspace(id: number): Promise<void>;
  reviewTargets(prId: number, teams: string[]): Promise<TargetChoice[]>;
  reviewPreview(prId: number, target: ReviewTarget): Promise<ReviewPreview>;
  sendReview(prId: number, target: ReviewTarget, teams: string[]): Promise<SlackIndicator>;
  bulkPreview(request: BulkRequest): Promise<BulkPreview>;
  sendBulk(request: BulkRequest, fingerprints: string[]): Promise<BulkResult>;
  importMapping(csv: string, workspace: string): Promise<{ rows: number }>;
  /** Native open dialog filtered to .csv (with All files); null when cancelled. */
  pickMappingFile(): Promise<MappingFile | null>;
  finish(): Promise<SyncStatus>;
  syncNow(): Promise<SyncStatus>;
  syncStatus(): Promise<SyncStatus>;
  listRows(): Promise<ListRow[]>;
  listProjects(): Promise<ProjectDto[]>;
  createProject(name: string): Promise<ProjectDto>;
  renameProject(id: number, name: string): Promise<void>;
  archiveProject(id: number, archived: boolean): Promise<void>;
  deleteProject(id: number): Promise<void>;
  setProjectColor(id: number, color: string): Promise<ProjectDto>;
  listProjectRules(projectId: number): Promise<ProjectRuleDto[]>;
  saveProjectRule(rule: ProjectRuleDto): Promise<ProjectRuleDto>;
  deleteProjectRule(id: number): Promise<void>;
  previewRule(kind: RuleKind, pattern: string): Promise<number>;
  listProjectLinks(projectId: number): Promise<ProjectLinkDto[]>;
  linkPrs(projectId: number, prIds: number[]): Promise<number>;
  unlinkPr(projectId: number, prId: number): Promise<void>;
  restorePr(projectId: number, prId: number): Promise<void>;
  loadViewState(viewKey: string): Promise<string | null>;
  saveViewState(viewKey: string, stateJson: string): Promise<void>;
  openExternal(url: string): Promise<void>;
  prDetail(id: number): Promise<PrDetail>;
  prSlackContext(prId: number, refresh: boolean): Promise<ContextThread[]>;
  prSlackMentions(prId: number): Promise<MentionGroup[]>;
  saveSlackMentions(scope: MentionsScope): Promise<void>;
  loadPreferences(): Promise<Preferences>;
  savePreferences(prefs: Preferences): Promise<Preferences>;
  listMappings(): Promise<MappingEntry[]>;
  saveMapping(row: MappingRow, disabled: boolean): Promise<void>;
  resetMapping(org: string, slug: string): Promise<boolean>;
  previewMappingImport(csv: string, workspace: string, mode: ImportMode): Promise<ImportPreview>;
  applyMappingImport(csv: string, workspace: string, mode: ImportMode): Promise<{ rows: number }>;
  exportMapping(mode: ExportMode): Promise<string>;
  appInfo(): Promise<AppInfo>;
  /** Latest GitHub release when it is newer than this build; null otherwise. */
  checkUpdate(): Promise<UpdateInfo | null>;
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
  slackWorkspaces: () => invoke("slack_workspaces"),
  slackConnect: () => invoke("slack_connect"),
  slackCancelConnect: () => invoke("slack_cancel_connect"),
  slackRemoveWorkspace: (id) => invoke("slack_remove_workspace", { id }),
  reviewTargets: (prId, teams) => invoke("review_targets", { prId, teams }),
  reviewPreview: (prId, target) => invoke("review_preview", { prId, target }),
  sendReview: (prId, target, teams) => invoke("send_review", { prId, target, teams }),
  bulkPreview: (request) => invoke("bulk_preview", { request }),
  sendBulk: (request, fingerprints) => invoke("send_bulk", { request, fingerprints }),
  importMapping: (csv, workspace) => invoke("import_mapping", { csv, workspace }),
  pickMappingFile: async () => {
    const path = await open({
      title: "Choose a mapping CSV",
      multiple: false,
      directory: false,
      filters: [
        { name: "CSV", extensions: ["csv"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    return path ? invoke<MappingFile>("read_mapping_file", { path }) : null;
  },
  finish: () => invoke("finish_onboarding"),
  syncNow: () => invoke("sync_now"),
  syncStatus: () => invoke("sync_status"),
  listRows: () => invoke("list_rows"),
  listProjects: () => invoke("list_projects"),
  createProject: (name) => invoke("create_project", { name }),
  renameProject: (id, name) => invoke("rename_project", { id, name }),
  archiveProject: (id, archived) => invoke("archive_project", { id, archived }),
  deleteProject: (id) => invoke("delete_project", { id }),
  setProjectColor: (id, color) => invoke("set_project_color", { id, color }),
  listProjectRules: (projectId) => invoke("list_project_rules", { projectId }),
  saveProjectRule: (rule) => invoke("save_project_rule", { rule }),
  deleteProjectRule: (id) => invoke("delete_project_rule", { id }),
  previewRule: (kind, pattern) => invoke("preview_rule", { kind, pattern }),
  listProjectLinks: (projectId) => invoke("list_project_links", { projectId }),
  linkPrs: (projectId, prIds) => invoke("link_prs", { projectId, prIds }),
  unlinkPr: (projectId, prId) => invoke("unlink_pr", { projectId, prId }),
  restorePr: (projectId, prId) => invoke("restore_pr", { projectId, prId }),
  loadViewState: (viewKey) => invoke("load_view_state", { viewKey }),
  saveViewState: (viewKey, stateJson) => invoke("save_view_state", { viewKey, stateJson }),
  openExternal: (url) => invoke("open_external", { url }),
  prDetail: (id) => invoke("pr_detail", { id }),
  prSlackContext: (prId, refresh) => invoke("pr_slack_context", { prId, refresh }),
  prSlackMentions: (prId) => invoke("pr_slack_mentions", { prId }),
  saveSlackMentions: (scope) => invoke("save_slack_mentions", { scope }),
  loadPreferences: () => invoke("load_preferences"),
  savePreferences: (prefs) => invoke("save_preferences", { prefs }),
  listMappings: () => invoke("list_mappings"),
  saveMapping: (row, disabled) => invoke("save_mapping", { row, disabled }),
  resetMapping: (org, slug) => invoke("reset_mapping", { org, slug }),
  previewMappingImport: (csv, workspace, mode) => invoke("preview_mapping_import", { csv, workspace, mode }),
  applyMappingImport: (csv, workspace, mode) => invoke("apply_mapping_import", { csv, workspace, mode }),
  exportMapping: (mode) => invoke("export_mapping", { mode }),
  appInfo: () => invoke("app_info"),
  checkUpdate: () => invoke("check_update"),
};

/**
 * How a channel reads in the UI: `#reviews`, or `DM` for a direct message
 * (IDs starting with D). DM user names are not resolved: that needs im:read.
 */
export function channelLabel(target: { channel_id: string; channel_name: string }): string {
  const name = target.channel_name.trim().replace(/^#/, "");
  if (target.channel_id.startsWith("D")) {
    return name && name !== target.channel_id ? `DM · ${name}` : "DM";
  }
  return `#${name || target.channel_id}`;
}

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
