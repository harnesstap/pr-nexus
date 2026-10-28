import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { PageDto, PageRequest } from "./list/paging";

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
  /** Bumps only when a sync changed stored rows; re-read the list on a change (§6). */
  revision: number;
  /** Live updates from the notifications inbox (§6). */
  live?: LiveStatus;
}

export interface LiveStatus {
  /** `active`, `starting`, `off`, `unavailable`, or `demo`. */
  state: string;
  /** Last answered inbox poll. */
  checked_at: string | null;
  /** Why it is unavailable, or the last error. */
  detail: string | null;
}

/** The keychain state (§9). `waiting` usually means a system prompt is on screen. */
export type KeyringState = "ready" | "waiting" | "timed_out" | "denied" | "no_backend" | "error" | "disabled";

export interface KeyringStatus {
  state: KeyringState;
  /** Banner text; null when the keychain is ready. */
  message: string | null;
  /** Retry can help (timed out, denied, no keychain, error). */
  can_retry: boolean;
  /** Some tokens are only in memory, not in the keychain yet. */
  unsaved: boolean;
}

export interface Bootstrap {
  onboarding: OnboardingState;
  resume_step: StepId;
  steps: StepId[];
  keyring: KeyringStatus;
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
  /** False after setup: `gh_ready`/`gh_detail` were skipped; call `probeGh`. */
  gh_probed: boolean;
}

export interface GhStatus {
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
  live_updates: boolean;
  /** JIRA site, e.g. `https://acme-labs.atlassian.net`; turns bare keys into links. Empty: none. */
  jira_base_url: string;
}

export interface MappingRow {
  github_org: string;
  team_slug: string;
  team_name: string;
  slack_workspace: string;
  /** Slack team (`T…`) that routes sends; empty until known (§7.11). */
  slack_team_id?: string;
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

/** A thread PR Nexus posted for this PR, with what Slack returned. */
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
  /** Slack team (`T…`): the routing key (§7.11). */
  team_id: string;
  /** Off: the grant stays, but nothing searches, sends, bumps or reads here. */
  enabled: boolean;
  /** 0 is tried first. */
  priority: number;
}

export interface ReviewTarget {
  /** Domain; only a hint, since a pasted link can carry the org host. */
  workspace: string;
  /** Slack team (`T…`) when known; routes before the domain. */
  team_id?: string | null;
  /** Team name for display; empty when unknown. */
  workspace_name?: string;
  channel_id: string;
  channel_name: string;
}

export interface TargetChoice {
  target: ReviewTarget;
  teams: string[];
  source: "request" | "mapping";
}

/** A target picker search result (§7.11): a channel, or a member who gets a DM. */
export interface TargetMatch {
  target: ReviewTarget;
  kind: "channel" | "member";
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
  owner: string;
  repo: string;
  author_login: string;
  head_ref: string;
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
  /** Starred by the user; pinned above the rest within the sort. */
  starred: boolean;
  project_ids: number[];
  readiness: "ready" | "blocked" | "unknown";
  readiness_notes: string[];
  /** GitHub reports merge conflicts. */
  conflicts: boolean;
  /** At least one reviewer or team is requested on GitHub. */
  reviewers_requested: boolean;
  /** Who still has to review: users first, then teams, each by login. */
  pending_reviewers: PendingReviewer[];
  slack: {
    requested_at: string;
    last_bumped_at: string | null;
    bump_count: number;
    workspace: string;
    /** Slack team (`T…`) the request lives in, when known. */
    team_id?: string | null;
    /** Team name of that workspace, else the domain. */
    workspace_name?: string;
    /** That workspace is turned off in Settings → Slack. */
    workspace_disabled?: boolean;
    channel_id: string;
    channel_name: string;
    permalink: string | null;
    root_ts: string;
    source: string;
  } | null;
}

/** One pending review request on a list row. */
export interface PendingReviewer {
  kind: "user" | "team";
  /** User login, or `org/slug` for a team. */
  login: string;
  /** Display name or team name, when GitHub returned one. */
  name: string | null;
  /** Avatar URL with an explicit size; null falls back to initials. */
  avatar_url: string | null;
}

export interface ProjectDto {
  id: number;
  name: string;
  color: string;
  archived: boolean;
  description: string;
  sort_order: number;
  /** JIRA references in display order. */
  jira_links: JiraLinkDto[];
}

/** One JIRA reference on a project. */
export interface JiraLinkDto {
  id: number;
  /** `ACME-123`, or empty for a link without a key. */
  key: string;
  /** The link as entered, or empty for a bare key. */
  url: string;
  /** Where the chip opens; null for a bare key while no JIRA base URL is set. */
  href: string | null;
}

/** A link as the editor sends it: text to parse (a key or a URL). */
export interface JiraLinkInput {
  text: string;
}

export interface PrNoteDto {
  pr_id: number;
  /** Empty when there is no note. */
  note: string;
  updated_at: string | null;
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

/** What one manual link replaced, so Undo can put it back (`before: null` = not linked). */
export interface ReplacedLinkDto {
  pr_id: number;
  before: "rule" | "excluded" | null;
  rule_id: number | null;
}

export interface LinkOutcomeDto {
  /** Rows that became manual links; PRs already linked by hand are not counted. */
  changed: number;
  undo: ReplacedLinkDto[];
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

export interface Explanation {
  tone: "good" | "warn" | "bad" | "info";
  text: string;
}

export type MergeMethod = "squash" | "merge" | "rebase";

export interface OverviewCheck {
  name: string;
  state: "passed" | "failed" | "pending" | "skipped";
  url: string | null;
  required: boolean;
}

/** A user or team; `login` is `org/slug` for a team. */
export interface Person {
  /** Bots are kept and marked, so a bot-authored PR still lists its author. */
  kind: "user" | "team" | "bot";
  login: string;
  name: string | null;
  avatar_url: string | null;
}

export interface OverviewReviewer extends Person {
  state: "approved" | "changes_requested" | "commented" | "pending" | "dismissed";
}

/** The detail panel's at-a-glance facts (§7.2), cached per PR. */
export interface PrOverview {
  state: "open" | "closed" | "merged";
  is_draft: boolean;
  base_ref: string;
  head_ref: string;
  mergeable: "mergeable" | "conflicting" | "unknown";
  /** GitHub's mergeStateStatus, lower case. */
  merge_state: "clean" | "blocked" | "behind" | "dirty" | "unstable" | "has_hooks" | "draft" | "unknown";
  review_decision: "approved" | "changes_requested" | "review_required" | null;
  issue_comments: number;
  review_comments: number;
  checks: OverviewCheck[];
  checks_total: number;
  approvals: number;
  /** Null when the token cannot see the branch rules. */
  required_approvals: number | null;
  reviewers: OverviewReviewer[];
  /** Null for a deleted account. */
  author: Person | null;
  assignees: Person[];
  /** Authors of PR and review-thread comments, deduped; bots included. */
  commenters: Person[];
  viewer: {
    login: string;
    permission: "admin" | "maintain" | "write" | "triage" | "read" | null;
    can_merge_as_admin: boolean;
    can_enable_auto_merge: boolean;
    can_disable_auto_merge: boolean;
  };
  merge_settings: { auto_merge_allowed: boolean; squash: boolean; merge: boolean; rebase: boolean };
  auto_merge: { method: MergeMethod; enabled_by: string | null; enabled_at: string | null } | null;
  fetched_at: string;
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
  code_owner_review_pending: boolean | null;
  mandatory_teams: string[];
  optional_teams: string[];
  approved_teams: string[];
  candidate_teams: string[];
  attribution_unknown: TeamGap[];
  requested_teams: string[];
  users_ignored: string[];
  /** Plain-language sentences for the Details expander, blockers first. */
  explanations: Explanation[];
  files: FileOwners[];
  files_truncated: boolean;
  rules_error: string | null;
  protection: string;
  from_cache: boolean;
}

export interface Api {
  bootstrap(): Promise<Bootstrap>;
  /** Never waits on the keychain; poll it while the state is `waiting` or `timed_out`. */
  keyringStatus(): Promise<KeyringStatus>;
  /** Ask the keychain again and save what is only in memory. Returns before the answer. */
  keyringRetry(): Promise<KeyringStatus>;
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
  /** Turn a workspace on or off for routing; returns the list in priority order. */
  slackSetWorkspaceEnabled(teamId: string, enabled: boolean): Promise<SlackWorkspace[]>;
  /** Save the routing priority, first to last; returns the list. */
  slackSetWorkspaceOrder(teamIds: string[]): Promise<SlackWorkspace[]>;
  reviewTargets(prId: number, teams: string[]): Promise<TargetChoice[]>;
  reviewPreview(prId: number, target: ReviewTarget): Promise<ReviewPreview>;
  /** `#term` channels, `@term` members, a bare term both; scoped to one workspace. */
  /** Searches every enabled workspace in priority order; merged and labeled. */
  searchSlackTargets(query: string): Promise<TargetMatch[]>;
  /** `note` is appended after a blank line; see `withNote`. */
  sendReview(prId: number, target: ReviewTarget, teams: string[], note?: string): Promise<SlackIndicator>;
  bulkPreview(request: BulkRequest): Promise<BulkPreview>;
  sendBulk(request: BulkRequest, fingerprints: string[]): Promise<BulkResult>;
  importMapping(csv: string, workspace: string): Promise<{ rows: number }>;
  /** Native open dialog filtered to .csv (with All files); null when cancelled. */
  pickMappingFile(): Promise<MappingFile | null>;
  finish(): Promise<SyncStatus>;
  syncNow(): Promise<SyncStatus>;
  syncStatus(): Promise<SyncStatus>;
  /** Re-read every open pull request instead of only what changed (§6). */
  fullResync(): Promise<SyncStatus>;
  /** One slice of the list with counts over every match (§8.2). */
  listPage(request: PageRequest): Promise<PageDto>;
  /** Rows by id in the order asked; unknown ids are skipped. */
  rowsById(ids: number[]): Promise<ListRow[]>;
  /** Open PRs matching `query` that the project does not link yet. */
  linkCandidates(projectId: number, query: string, limit: number): Promise<ListRow[]>;
  /** Whether the gh CLI has a token (wizard GitHub step). */
  probeGh(): Promise<GhStatus>;
  listProjects(): Promise<ProjectDto[]>;
  createProject(name: string): Promise<ProjectDto>;
  /** Creates a project and links the PRs by hand in one transaction. */
  createProjectWithPrs(name: string, prIds: number[]): Promise<ProjectDto>;
  renameProject(id: number, name: string): Promise<void>;
  archiveProject(id: number, archived: boolean): Promise<void>;
  deleteProject(id: number): Promise<void>;
  setProjectColor(id: number, color: string): Promise<ProjectDto>;
  /** Multi-line description; blank clears it. */
  setProjectDescription(projectId: number, description: string): Promise<ProjectDto>;
  addJiraLink(projectId: number, link: JiraLinkInput): Promise<JiraLinkDto>;
  removeJiraLink(linkId: number): Promise<void>;
  /** Replaces every link in order: adds, removes and reorders at once. */
  setJiraLinks(projectId: number, links: JiraLinkInput[]): Promise<JiraLinkDto[]>;
  prNote(prId: number): Promise<PrNoteDto>;
  /** Saves the note; blank deletes it. */
  setPrNote(prId: number, note: string): Promise<PrNoteDto>;
  /** Idempotent; resolves to the new state. */
  setPrStarred(prId: number, starred: boolean): Promise<boolean>;
  listProjectRules(projectId: number): Promise<ProjectRuleDto[]>;
  saveProjectRule(rule: ProjectRuleDto): Promise<ProjectRuleDto>;
  deleteProjectRule(id: number): Promise<void>;
  previewRule(kind: RuleKind, pattern: string): Promise<number>;
  listProjectLinks(projectId: number): Promise<ProjectLinkDto[]>;
  /** All or nothing; the outcome carries what Undo needs. */
  linkPrs(projectId: number, prIds: number[]): Promise<LinkOutcomeDto>;
  /** Reverts a `linkPrs` outcome; rows changed since then are left alone. */
  undoLinkPrs(projectId: number, undo: ReplacedLinkDto[]): Promise<number>;
  unlinkPr(projectId: number, prId: number): Promise<void>;
  restorePr(projectId: number, prId: number): Promise<void>;
  loadViewState(viewKey: string): Promise<string | null>;
  saveViewState(viewKey: string, stateJson: string): Promise<void>;
  openExternal(url: string): Promise<void>;
  prDetail(id: number): Promise<PrDetail>;
  /** The detail panel's cards. `refresh: false` reads only the cache (null when empty). */
  prOverview(id: number, refresh: boolean): Promise<PrOverview | null>;
  /** Turns auto-merge on with `method`, or off with null; returns the fresh overview. */
  setAutoMerge(id: number, method: MergeMethod | null): Promise<PrOverview>;
  /** Converts to a draft (`true`) or marks ready for review (`false`); returns the fresh overview. */
  setPrDraft(id: number, draft: boolean): Promise<PrOverview>;
  /** Merges now with `method`; irreversible, so the UI confirms first. Returns the fresh overview. */
  mergePr(id: number, method: MergeMethod): Promise<PrOverview>;
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
  keyringStatus: () => invoke("keyring_status"),
  keyringRetry: () => invoke("keyring_retry"),
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
  slackSetWorkspaceEnabled: (teamId, enabled) => invoke("slack_set_workspace_enabled", { teamId, enabled }),
  slackSetWorkspaceOrder: (teamIds) => invoke("slack_set_workspace_order", { teamIds }),
  reviewTargets: (prId, teams) => invoke("review_targets", { prId, teams }),
  reviewPreview: (prId, target) => invoke("review_preview", { prId, target }),
  searchSlackTargets: (query) => invoke("slack_search_targets", { query }),
  sendReview: (prId, target, teams, note) => invoke("send_review", { prId, target, teams, note: note?.trim() ? note : null }),
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
  fullResync: () => invoke("full_resync"),
  listPage: (request) => invoke("list_page", { request }),
  rowsById: (ids) => invoke("rows_by_id", { ids }),
  linkCandidates: (projectId, query, limit) => invoke("link_candidates", { projectId, query, limit }),
  probeGh: () => invoke("probe_gh"),
  listProjects: () => invoke("list_projects"),
  createProject: (name) => invoke("create_project", { name }),
  createProjectWithPrs: (name, prIds) => invoke("create_project_with_prs", { name, prIds }),
  renameProject: (id, name) => invoke("rename_project", { id, name }),
  archiveProject: (id, archived) => invoke("archive_project", { id, archived }),
  deleteProject: (id) => invoke("delete_project", { id }),
  setProjectColor: (id, color) => invoke("set_project_color", { id, color }),
  setProjectDescription: (projectId, description) => invoke("set_project_description", { projectId, description }),
  addJiraLink: (projectId, link) => invoke("add_jira_link", { projectId, link }),
  removeJiraLink: (linkId) => invoke("remove_jira_link", { linkId }),
  setJiraLinks: (projectId, links) => invoke("set_jira_links", { projectId, links }),
  prNote: (prId) => invoke("pr_note", { prId }),
  setPrNote: (prId, note) => invoke("set_pr_note", { prId, note }),
  setPrStarred: (prId, starred) => invoke("set_pr_starred", { prId, starred }),
  listProjectRules: (projectId) => invoke("list_project_rules", { projectId }),
  saveProjectRule: (rule) => invoke("save_project_rule", { rule }),
  deleteProjectRule: (id) => invoke("delete_project_rule", { id }),
  previewRule: (kind, pattern) => invoke("preview_rule", { kind, pattern }),
  listProjectLinks: (projectId) => invoke("list_project_links", { projectId }),
  linkPrs: (projectId, prIds) => invoke("link_prs", { projectId, prIds }),
  undoLinkPrs: (projectId, undo) => invoke("undo_link_prs", { projectId, undo }),
  unlinkPr: (projectId, prId) => invoke("unlink_pr", { projectId, prId }),
  restorePr: (projectId, prId) => invoke("restore_pr", { projectId, prId }),
  loadViewState: (viewKey) => invoke("load_view_state", { viewKey }),
  saveViewState: (viewKey, stateJson) => invoke("save_view_state", { viewKey, stateJson }),
  openExternal: (url) => invoke("open_external", { url }),
  prDetail: (id) => invoke("pr_detail", { id }),
  prOverview: (id, refresh) => invoke("pr_overview", { id, refresh }),
  setAutoMerge: (id, method) => invoke("set_auto_merge", { id, method }),
  setPrDraft: (id, draft) => invoke("set_pr_draft", { id, draft }),
  mergePr: (id, method) => invoke("merge_pr", { id, method }),
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
 * How a channel reads in the UI: `#reviews`, `DM` for a direct message
 * (IDs starting with D), or `@name` for a member (U or W), who gets a DM.
 * DM user names are not resolved: that needs im:read.
 */
export function channelLabel(target: { channel_id: string; channel_name: string }): string {
  const name = target.channel_name.trim().replace(/^[#@]/, "");
  if (/^[UW]/.test(target.channel_id)) {
    return `@${name || target.channel_id}`;
  }
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
