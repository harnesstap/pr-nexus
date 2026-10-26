# PR-Nexus — Technical Specification (v1)

Status: v1 spec, 2026-10-25 (open questions answered; see §14) · Owner: Christophe Oudar · Audience: coding agents bootstrapping this repo, and human reviewers.

This file is the source of truth for product behavior. When code and spec disagree, fix one of them in the same PR. Sections marked **MUST** are acceptance-relevant. Everything marked *Future* is out of scope for v1. Do not build it, but do not design it out either.

---

## 1. Vision & non-goals

**Vision.** A desktop app for one developer who has many open pull requests across one or more GitHub orgs that they choose. It answers two questions in one place: *"what state are my PRs in, grouped by the project they belong to?"* and *"who do I ping on Slack to get them reviewed?"* It also performs that ping (request, or bump an earlier request) in one click, with a preview before anything is sent.

PR-Nexus reimplements, natively in Rust, the workflow of the existing Claude skill `request-slack-review` (§7). That skill is the behavioral reference. Deviations from it are listed explicitly.

**Non-goals (v1)**
- Not a code review tool. No diff viewer, no commenting, no approving.
- Not a Slack client. It never syncs whole channels. It reads only messages tied to a PR (§7.9).
- Not a team dashboard. It serves one user, holds local state, and has no server.
- No webhooks and no background daemon when the app is closed.
- No AI features in v1. The MCP server is *Future* (§10.1).

## 2. Scope: v1 vs later

| Area | v1 | Later |
|---|---|---|
| Platforms | Desktop. **macOS first** (Apple Silicon). Linux and Windows **must build in CI** but are not shipped or tested by hand. | Android/iOS (Tauri mobile) |
| VCS | GitHub.com: read (PRs, checks, reviews, CODEOWNERS, rules). One optional write (see note). | GitLab, Gitea, GHES |
| Chat | Slack, **N workspaces in v1**, chosen by the user: read (threads tied to PRs, optional mention search) and write (post request, post bump) | Discord, Mattermost |
| Views | Grouped list (by project) with multi-select and bulk actions; PR detail; projects mgmt; settings (accounts, mappings, rules) | Kanban, stats |
| Automation | Polling sync while the app runs | Scheduled reminders, MCP server |

Note on "GitHub write": the user asked for read and write. No GitHub mutation is needed by the v1 features. The only one that fits naturally is **"also request the owning team as reviewer on GitHub"** (`requestReviews` with `teamIds`). It is an opt-in checkbox in the request preview (default off), in milestone M10. Without it, v1 performs no GitHub writes.

Traits for VCS and chat providers stay generic (§5), so other providers can be added later. Only GitHub and Slack are implemented.

## 3. Architecture & workspace layout

```
pr-nexus/
├─ Cargo.toml                    # [workspace] members = crates/*, app/src-tauri; [workspace.package] version
├─ crates/
│  ├─ pr-nexus-core/             # domain types, errors, provider traits, SQLite store + migrations,
│  │                             # sync engine, ownership/CODEOWNERS, mapping, message builder,
│  │                             # review-request planner, project auto-link rules. NO network deps.
│  └─ pr-nexus-providers/        # GitHub (octocrab + GraphQL) and Slack (thin reqwest client)
│                                # implementations of core traits; auth helpers (keyring, gh CLI).
├─ app/
│  ├─ package.json, vite.config.ts, src/ (React + TS + Tailwind)
│  └─ src-tauri/                 # bin crate `pr-nexus`: Tauri commands, app state, poll scheduler
├─ docs/SPEC.md, AGENTS.md, CONTRIBUTING.md
├─ .changes/ (Changie), .changie.yaml, CHANGELOG.md
└─ .github/workflows/ ci.yml, nightly.yml, tag-release.yml, release.yml
```

**Why 2 library crates plus the app (and not the original 3–4):**
- `core` contains all logic that can be tested without the network: ownership rules, mapping precedence, message text, planner, SQL. It compiles fast and has no TLS stack. That keeps the edit-test loop for agents short.
- `providers` isolates heavy dependencies (octocrab, reqwest, rustls, keyring) and HTTP fixtures. It implements the traits from `core`.
- `app/src-tauri` stays a thin layer: it wires state and maps commands to services. It is a workspace member, so `cargo test --workspace` covers it.
- There is no `pr-nexus-mcp` crate in v1 (YAGNI). §10.1 describes where it will go.
- Splitting `adapters` into `vcs` and `chat` crates is rejected: 2 implementations do not justify 2 crates.

**Runtime model**
- A single Tauri process. One `tokio` runtime (Tauri's).
- Store: one SQLite file at `$APPDATA/pr-nexus/pr-nexus.db`, in WAL mode, `foreign_keys=ON`, `busy_timeout=5000`. Access goes through a `Store` that owns one write connection on a dedicated thread (actor, `tokio-rusqlite`-style) plus a small read pool. WAL already prepares for a second process, the *Future* MCP server.
- The frontend never runs SQL. Everything goes through typed Tauri commands. TS types are generated with `tauri-specta`. If the specta crates are unstable at bootstrap time, hand-write `app/src/bindings.ts` and add a test that keeps it in sync. Push events (`sync://updated`, `review://progress`) use Tauri events.
- `tauri-plugin-sql` is rejected, because it exposes raw SQL to the webview.

**Library choices (versions checked on crates.io, 2026-10-25)**

| Need | Choice | Version | Rationale |
|---|---|---|---|
| Desktop shell | `tauri` v2 | 2.12.x (CLI 2.12.4) | Stable v2. Capabilities-based permissions. |
| SQLite | `rusqlite` (`bundled`) + `rusqlite_migration` | 0.41 / 2.6 | Sync API with simple ownership. sqlx 0.9 adds compile-time query checking (needs `DATABASE_URL`/offline prepare) and async-over-threadpool for SQLite, which buys nothing here. |
| GitHub | `octocrab` | 0.54.x | Auth, REST models, and `Octocrab::graphql()` passthrough. GraphQL queries are hand-written `.graphql` strings with serde response structs. `graphql_client` codegen (0.16) is rejected to avoid vendoring the ~1 MB schema. Fixture tests guard the shapes. |
| Slack | own thin `reqwest` client | — | Needs ~8 methods (§7.10). `slack-morphism` 2.31 is capable but large. |
| Secrets | `keyring` 4.x, default `v1` feature (same `Entry` API as v1) | 4.2 | v4 wraps `keyring-core` plus per-OS stores. If finer control is needed, use `keyring-core` + `apple-native-keyring-store` / `windows-native-keyring-store` / `zbus-secret-service-keyring-store`. |
| Errors | `thiserror` | 2.x | §4.3 |
| CSV | `csv` | 1.4 | mapping import/export |
| Frontend | React + TypeScript + Vite + Tailwind + TanStack Table + TanStack Query | — | Multi-select tables are the core UI. TanStack Table handles grouping and row selection. |
| JS tooling | Bun 1.3 (as in `harnesstap`) | — | Same toolchain as the user's other repo |
| *Future* MCP | `rmcp` (official Rust SDK) | 3.7 | §10.1 |

## 4. Domain model & SQLite schema

### 4.1 Rust types (pr-nexus-core::domain)

```rust
// ---- identity -----------------------------------------------------------
pub enum VcsKind { GitHub /* GitLab, Gitea later */ }
pub enum ChatKind { Slack /* Discord, Mattermost later */ }

pub struct AccountId(pub i64);          // row in `accounts`
pub struct PrId(pub i64);               // local surrogate key
pub struct ProjectId(pub i64);

pub struct RepoRef { pub host: String, pub owner: String, pub name: String } // host = "github.com"
pub struct TeamRef { pub org: String, pub slug: String } // org kept: CODEOWNERS may name another org's team

/// Opaque provider handle. Core never parses `id`.
/// GitHub: GraphQL node id. Slack message: "C123:1700000000.000100".
pub struct ProviderRef { pub account: AccountId, pub id: String }

// ---- pull requests --------------------------------------------------------
pub enum PrState { Open, Closed, Merged }                 // isDraft is a separate bool
pub enum ReviewDecision { Approved, ChangesRequested, ReviewRequired }
pub enum ChecksState { Success, Failure, Pending, Error, Expected, None } // statusCheckRollup.state; None = no checks
pub enum Mergeable { Mergeable, Conflicting, Unknown }
pub enum MergeStateStatus { Clean, Behind, Blocked, Dirty, Draft, HasHooks, Unstable, Unknown }
pub enum Reviewer { User { login: String }, Team(TeamRef) }
bitflags! { pub struct Involvement: u8 { const AUTHOR = 1; const REVIEW_REQUESTED = 2; } }

pub struct PullRequest {
    pub id: PrId, pub remote: ProviderRef, pub repo: RepoRef, pub number: u64,
    pub title: String, pub url: String, pub author_login: String,
    pub body: String,                         // bodyText, capped at 8 KB; used only by the search bar (§8.2)
    pub state: PrState, pub is_draft: bool,
    pub head_ref: String, pub base_ref: String, pub head_sha: String,
    pub labels: Vec<String>,
    pub review_decision: Option<ReviewDecision>,
    pub checks: ChecksState, pub mergeable: Mergeable, pub merge_state: MergeStateStatus,
    pub requested_reviewers: Vec<Reviewer>,   // informational only, NEVER ownership (§7.2)
    pub unresolved_threads: u32, pub total_threads: u32,
    pub involvement: Involvement,
    pub created_at: DateTime<Utc>, pub updated_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,      // max(updatedAt, last commit, last review)
    pub synced_at: DateTime<Utc>,
}

// ---- ownership (computed, cached per head_sha) ----------------------------
pub enum Tri { True, False, Unknown }
pub enum Readiness { Ready, Blocked { reasons: Vec<String> }, Unknown { reasons: Vec<String> } }
pub struct OwnerDecision { /* fields and rules in §7.2 */ }

// ---- chat -----------------------------------------------------------------
pub struct ChatMessage {
    pub remote: ProviderRef, pub channel_id: String,
    pub thread_root: Option<ProviderRef>,     // None = top-level
    pub author: String, pub text: String, pub posted_at: DateTime<Utc>,
    pub reply_count: u32, pub permalink: Option<String>,
}

// ---- review requests (source of truth for "is this PR already requested?", §7.4) ----
pub struct ReviewRequestId(pub i64);
pub enum RequestSource { App, DiscoveredViaSearch }
pub enum RequestStatus { Active, Closed(CloseReason) }
pub enum CloseReason { PrMerged, PrClosed, Dismissed, Superseded, ChannelArchived }
pub struct ReviewRequest {
    pub id: ReviewRequestId, pub pr: PrId,
    pub workspace: String, pub channel_id: String, pub channel_name: String,
    pub root: ProviderRef,                    // the top-level request message (bumps reply in its thread)
    pub permalink: Option<String>,
    pub requested_at: DateTime<Utc>, pub last_bumped_at: Option<DateTime<Utc>>, pub bump_count: u32,
    pub status: RequestStatus, pub source: RequestSource,
}
/// The single contextual Slack action for a PR (§7.7). Derived, never stored.
pub enum SlackAction {
    Request,                                   // no active request for this PR
    Bump { requested_at: DateTime<Utc>, last_bumped_at: Option<DateTime<Utc>> },
    Disabled { reason: String },               // blocked, all owners approved, not your PR, …
}

// ---- projects -------------------------------------------------------------
pub struct Project { pub id: ProjectId, pub name: String, pub description: String,
                     pub color: String, pub archived: bool, pub sort_order: i32 }
pub enum RuleKind { TicketKeyInBranch, BranchRegex, Label, Repo, TitleRegex }
pub struct ProjectRule { pub id: i64, pub project: ProjectId, pub kind: RuleKind, pub pattern: String, pub priority: i32 }
pub enum LinkSource { Manual, Rule(i64), Excluded } // Excluded = user removed an auto-link; rules must not re-add it
```

These changes fix the original design doc:
- `provider_type: String` and `review_status: String` become enums.
- `thread_ts` leaves the generic model; the Slack handle lives inside `ProviderRef.id`.
- PR↔project links have a single source of truth: the `pr_projects` join table. `GenericPR.linked_project_id` and `ProjectFeatureMapping.pr_ids` are both removed.
- `chat_channel_ids` on projects is dropped. Channels come from team ownership, not from projects.
- Review requests become a first-class entity (`ReviewRequest`, `review_requests` table), so the app always knows whether a PR has already been requested. The original doc only had a `thread_ts` on the PR.
- New PR fields: body (for search), CI rollup, requested reviewers/teams, head/base, labels, `is_draft` separate from state, mergeability, last activity, and owning teams (via the ownership cache).

**PR ↔ project cardinality: many-to-many.** A PR can belong to several projects (e.g. a shared-lib PR for two initiatives). In the grouped list it shows in each section, with a `+1` badge. Selection is keyed by `PrId`, so bulk actions dedupe automatically.

### 4.2 SQLite schema (migration 0001)

```sql
CREATE TABLE accounts (
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL CHECK (kind IN ('github','slack')),
  display_name TEXT NOT NULL, host TEXT NOT NULL,          -- github.com | <workspace>.slack.com
  external_id TEXT NOT NULL,                               -- GitHub user node id | Slack "T…:U…"
  slack_workspace TEXT UNIQUE,                             -- Slack accounts only: workspace domain (one account per workspace)
  login TEXT NOT NULL,
  auth_mode TEXT NOT NULL CHECK (auth_mode IN ('gh_cli','pat','oauth_device','slack_user_token')),
  keychain_key TEXT,                                       -- NULL for gh_cli (token read from `gh` each time)
  created_at TEXT NOT NULL, UNIQUE (kind, external_id));

CREATE TABLE repositories (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  owner TEXT NOT NULL, name TEXT NOT NULL, node_id TEXT, default_branch TEXT,
  archived INTEGER NOT NULL DEFAULT 0, UNIQUE (account_id, owner, name));

CREATE TABLE pull_requests (
  id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
  number INTEGER NOT NULL, node_id TEXT NOT NULL UNIQUE, title TEXT NOT NULL, url TEXT NOT NULL,
  author_login TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', state TEXT NOT NULL, is_draft INTEGER NOT NULL,
  head_ref TEXT NOT NULL, base_ref TEXT NOT NULL, head_sha TEXT NOT NULL,
  review_decision TEXT, checks_state TEXT NOT NULL, mergeable TEXT NOT NULL, merge_state TEXT NOT NULL,
  unresolved_threads INTEGER NOT NULL, total_threads INTEGER NOT NULL, involvement INTEGER NOT NULL,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_activity_at TEXT NOT NULL, synced_at TEXT NOT NULL,
  hidden INTEGER NOT NULL DEFAULT 0, UNIQUE (repo_id, number));
CREATE TABLE pr_labels (pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  name TEXT NOT NULL, PRIMARY KEY (pr_id, name));
CREATE TABLE pr_requested_reviewers (pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('user','team')), org TEXT, login_or_slug TEXT NOT NULL,
  PRIMARY KEY (pr_id, kind, login_or_slug));

CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT NOT NULL DEFAULT '',
  color TEXT NOT NULL DEFAULT '#64748b', archived INTEGER NOT NULL DEFAULT 0, sort_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE project_rules (id INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, pattern TEXT NOT NULL, priority INTEGER NOT NULL DEFAULT 0);
CREATE TABLE pr_projects (
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK (source IN ('manual','rule','excluded')),
  rule_id INTEGER REFERENCES project_rules(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL, PRIMARY KEY (pr_id, project_id));

-- team → channel mapping, layered (§7.5). Starts empty. Imported rows are never edited in place.
CREATE TABLE team_channel_mappings (
  id INTEGER PRIMARY KEY, layer TEXT NOT NULL CHECK (layer IN ('imported','user')),
  github_org TEXT NOT NULL, team_slug TEXT NOT NULL, team_name TEXT NOT NULL DEFAULT '',
  slack_workspace TEXT NOT NULL,                           -- Slack workspace domain, e.g. 'acme' (from acme.slack.com)
  slack_channel_id TEXT NOT NULL DEFAULT '', slack_channel_name TEXT NOT NULL DEFAULT '',
  disabled INTEGER NOT NULL DEFAULT 0,                     -- user tombstone: "never post for this team"
  note TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL,
  UNIQUE (layer, github_org, team_slug));
CREATE UNIQUE INDEX tcm_name ON team_channel_mappings(layer, github_org, team_name) WHERE team_name <> '';

-- channel picker cache, scoped per workspace (refreshed daily or on demand)
CREATE TABLE slack_channels_cache (slack_workspace TEXT NOT NULL, channel_id TEXT NOT NULL,
  name TEXT NOT NULL, is_private INTEGER NOT NULL, is_archived INTEGER NOT NULL, is_member INTEGER NOT NULL,
  fetched_at TEXT NOT NULL, PRIMARY KEY (slack_workspace, channel_id));

CREATE TABLE ownership_cache (pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  head_sha TEXT NOT NULL, base_ref TEXT NOT NULL, codeowners_oid TEXT,
  decision_json TEXT NOT NULL, computed_at TEXT NOT NULL);
CREATE TABLE team_members_cache (github_org TEXT NOT NULL, team_slug TEXT NOT NULL,
  members_json TEXT NOT NULL, complete INTEGER NOT NULL, fetched_at TEXT NOT NULL,
  PRIMARY KEY (github_org, team_slug));

-- Message log: every Slack message PR-Nexus posted or discovered. One message can cover several PRs (batch).
-- It feeds the context panel and auditing. Request state lives in review_requests, not here.
CREATE TABLE review_messages (
  id INTEGER PRIMARY KEY, slack_account_id INTEGER NOT NULL REFERENCES accounts(id),
  channel_id TEXT NOT NULL, channel_name TEXT NOT NULL, ts TEXT NOT NULL,
  thread_root_ts TEXT,                                     -- NULL = top-level request; set for bumps
  kind TEXT NOT NULL CHECK (kind IN ('request','bump')),
  origin TEXT NOT NULL CHECK (origin IN ('posted','imported')),
  permalink TEXT, text TEXT NOT NULL, batch_id TEXT, posted_at TEXT NOT NULL,
  UNIQUE (slack_account_id, channel_id, ts));
CREATE TABLE review_message_prs (
  message_id INTEGER NOT NULL REFERENCES review_messages(id) ON DELETE CASCADE,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  head_sha TEXT, teams_json TEXT NOT NULL DEFAULT '[]', PRIMARY KEY (message_id, pr_id));

-- Source of truth: does this PR have a review request, where, and how old is it (§7.4).
-- One row per (PR, workspace, channel, root message). A batch message creates one row per PR.
CREATE TABLE review_requests (
  id INTEGER PRIMARY KEY,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  slack_workspace TEXT NOT NULL, channel_id TEXT NOT NULL, channel_name TEXT NOT NULL,
  root_ts TEXT NOT NULL, permalink TEXT,
  requested_at TEXT NOT NULL, last_bumped_at TEXT, bump_count INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL CHECK (status IN ('active','closed')),
  close_reason TEXT CHECK (close_reason IN ('pr_merged','pr_closed','dismissed','superseded','channel_archived')),
  closed_at TEXT,
  source TEXT NOT NULL CHECK (source IN ('app','discovered_via_search')),
  CHECK ((status = 'active') = (close_reason IS NULL)),
  UNIQUE (pr_id, slack_workspace, channel_id, root_ts));
CREATE UNIQUE INDEX rr_one_active ON review_requests(pr_id, slack_workspace, channel_id) WHERE status = 'active';
CREATE TABLE discovery_runs (pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  slack_workspace TEXT NOT NULL, ran_at TEXT NOT NULL, outcome TEXT NOT NULL,   -- found | none | skipped_no_scope | error
  PRIMARY KEY (pr_id, slack_workspace));
CREATE TABLE send_attempts (id INTEGER PRIMARY KEY, batch_id TEXT NOT NULL, pr_id INTEGER, channel_id TEXT,
  origin TEXT NOT NULL DEFAULT 'ui', outcome TEXT NOT NULL CHECK (outcome IN ('sent','skipped','failed')),
  detail TEXT NOT NULL, at TEXT NOT NULL);

CREATE TABLE chat_thread_cache (slack_account_id INTEGER NOT NULL, channel_id TEXT NOT NULL,
  thread_root_ts TEXT NOT NULL, messages_json TEXT NOT NULL, fetched_at TEXT NOT NULL,
  PRIMARY KEY (slack_account_id, channel_id, thread_root_ts));

CREATE TABLE sync_state (key TEXT PRIMARY KEY, value_json TEXT NOT NULL, updated_at TEXT NOT NULL); -- watermarks, rate limits
CREATE TABLE settings (key TEXT PRIMARY KEY, value_json TEXT NOT NULL);                            -- §7.12
-- Per-view grouping, ordering, visible columns, filters (§8.2). view_key: 'mine' | 'to_review' | 'project:<id>'
CREATE TABLE view_state (view_key TEXT PRIMARY KEY, state_json TEXT NOT NULL, updated_at TEXT NOT NULL);
```

Rules:
- Timestamps are RFC 3339 UTC text.
- Enums are stored as lowercase snake_case text and mapped with `FromStr`/`Display`. Unknown values fail loudly.
- Migrations live in `crates/pr-nexus-core/migrations/NNNN_*.sql` and run at startup in one transaction.
- **No secrets in SQLite** (§9). A test asserts that no column name matches `token|secret|password`.

### 4.3 Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("rate limited until {reset_at}")] RateLimited { reset_at: DateTime<Utc>, secondary: bool },
    #[error("unauthorized: {0}")]  Unauthorized(String),  // 401, Slack invalid_auth / token_revoked
    #[error("forbidden: {0}")]     Forbidden(String),     // 403 non-rate-limit, SSO not authorized, missing_scope
    #[error("not found: {0}")]     NotFound(String),
    #[error("network: {0}")]       Network(String),       // timeouts, DNS, TLS
    #[error("api error {code}: {message}")] Api { code: String, message: String },
    #[error("decode: {0}")]        Decode(String),
}
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)] Provider(#[from] ProviderError),
    #[error("store: {0}")] Store(String),
    #[error("validation: {0}")] Validation(String),
    #[error("gate: {0}")] Gate(String),
}
```

Tauri commands return `Result<T, AppError>`, where `AppError` serializes to `{kind, message, retry_at?}`. The UI shows:
- a reauth banner on `Unauthorized`;
- a countdown on `RateLimited`;
- an SSO authorization link on `Forbidden` when GitHub sends `X-GitHub-SSO`.

## 5. Provider traits (pr-nexus-core::providers)

```rust
#[async_trait]
pub trait VcsProvider: Send + Sync {
    fn kind(&self) -> VcsKind;
    async fn viewer(&self) -> Result<VcsUser, ProviderError>;
    /// Open PRs the viewer authored or is requested on. Replaces org → repo → PR enumeration.
    async fn search_pull_requests(&self, q: &PrQuery) -> Result<Vec<PullRequest>, ProviderError>;
    /// Refresh known PRs by provider id (catches merged/closed PRs that left the search results).
    async fn refresh_pull_requests(&self, ids: &[ProviderRef]) -> Result<Vec<PullRequest>, ProviderError>;
    /// Everything ownership resolution needs, for one PR (§6, §7.2).
    async fn ownership_inputs(&self, pr: &PullRequest) -> Result<OwnershipInputs, ProviderError>;
    async fn team_members(&self, team: &TeamRef) -> Result<TeamMembers, ProviderError>;
    async fn rate_limit(&self) -> Result<RateBudget, ProviderError>;
    /// Optional write (M10): request team reviewers on the PR.
    async fn request_team_review(&self, pr: &PullRequest, teams: &[TeamRef]) -> Result<(), ProviderError>;
}

#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn kind(&self) -> ChatKind;
    async fn whoami(&self) -> Result<ChatUser, ProviderError>;
    async fn list_channels(&self, cursor: Option<String>) -> Result<Page<ChatChannel>, ProviderError>; // picker
    async fn channel_info(&self, channel_id: &str) -> Result<ChatChannel, ProviderError>;              // archived? name?
    async fn post(&self, msg: &OutgoingMessage) -> Result<ChatMessage, ProviderError>;
    async fn thread(&self, channel_id: &str, root: &ProviderRef) -> Result<Vec<ChatMessage>, ProviderError>;
    async fn search(&self, query: &str) -> Result<Vec<ChatMessage>, ProviderError>; // Forbidden if unsupported
    async fn permalink(&self, msg: &ProviderRef) -> Result<String, ProviderError>;
}

pub struct OutgoingMessage {
    pub channel_id: String, pub text: String,
    pub thread_root: Option<ProviderRef>,
    pub also_to_channel: bool,   // Slack: reply_broadcast. Ignored when thread_root is None.
    pub unfurl_links: bool,
}
```

All methods return typed errors (no `String`). The original `list_repositories` is dropped: navigation is project-first (§8), and repos are discovered from PR search results. `list_organizations` is kept only to fill the org picker in onboarding/Settings (`viewer.organizations`, plus orgs seen in search results), not for sync. Test doubles (`FakeVcs`, `FakeChat`) live in `pr-nexus-core::testing` behind a `testing` feature.

## 6. GitHub sync

**Discovery uses GraphQL search, not org→repo enumeration.** The original design walked orgs, then repos, then PRs. That costs O(repos) calls and misses PRs in repos the user forgot to configure. Instead, each GitHub account runs two search buckets:

| Bucket | Query | Notes |
|---|---|---|
| `mine` | `is:pr is:open author:@me archived:false sort:updated-desc` | primary |
| `review_requested` | `is:pr is:open review-requested:@me archived:false sort:updated-desc` | "To review" filter, shipped in v1 (default D4) |

- Use `search(type: ISSUE, query:…, first: 50)` with cursor pagination and a hard cap of 500 per bucket. `ISSUE_ADVANCED` was removed on 2025-09-04, so do not use it.
- **Org filtering happens client-side.** Several `org:` qualifiers separated by spaces are ANDed, so `org:acme org:acme-labs` matches nothing. The search runs unfiltered, and results are kept only for the orgs the user tracks (`github.tracked_orgs`, picked in onboarding and Settings → Accounts; §8). Nothing is hardcoded and there is no default org. Until the user picks orgs, the PR list shows an empty state with a "Choose orgs to track" button. Personal (user-owned) repos are a separate opt-in toggle. In the UI the org list is also a filter chip, not a navigation level.
- **PRs that disappear** from a bucket (merged, closed, or no longer requested) are refreshed once via `nodes(ids:[…])` so their final state is stored. Merged/closed PRs stay visible for `settings.sync.keep_closed_days` (default 7), then become `hidden`.

**Fragment** (one GraphQL file per query under `crates/pr-nexus-providers/src/github/graphql/`):

```graphql
fragment PrFields on PullRequest {
  id number title url bodyText isDraft state createdAt updatedAt
  author { login } headRefName baseRefName headRefOid
  repository { id owner { login } name isArchived defaultBranchRef { name } }
  labels(first: 20) { nodes { name } }
  reviewDecision mergeable mergeStateStatus
  commits(last: 1) { nodes { commit { committedDate statusCheckRollup { state } } } }
  reviewRequests(first: 20) { nodes { requestedReviewer {
    ... on User { login } ... on Team { slug organization { login } } } } }
  reviewThreads(first: 100) { totalCount nodes { isResolved } }
  latestOpinionatedReviews(first: 50) { nodes { state author { login __typename } submittedAt
    onBehalfOf(first: 10) { nodes { slug organization { login } } } } }
}
```

Unresolved threads come from counting `reviewThreads.nodes[].isResolved == false`. With more than 100 threads, show "100+".

**Polling** (no webhooks: a desktop app has no public endpoint):
- every 3 min while the window is focused, every 15 min while unfocused or minimised, and never while the app is quit;
- immediately when a PR detail view opens (that one PR, plus its ownership inputs if `head_sha` changed);
- **always** right before any Slack write for the affected PRs (§7.8);
- the manual refresh button in the sidebar footer (§8.2), which also shows sync status: `Synced · 2m ago`, `Syncing…`, or an error such as `Rate limited · resets 14:32` (time from `resetAt`, shown in local time) or `Sync failed · <reason>`.

**Rate limits.**
- Read `rateLimit { remaining resetAt cost }` in every GraphQL query and the REST `x-ratelimit-*` headers.
- Store them in `sync_state`.
- When remaining is below 10% of the limit, skip scheduled polls until `resetAt`; manual and pre-send refreshes still run.
- On a secondary limit (403/429 with `retry-after`), return `RateLimited{secondary:true}` and back off exponentially starting at `retry-after`.
- Send at most 2 concurrent requests per host.

**Ownership inputs.** These are fetched lazily, only for PRs the user opens or selects for a request. They are cached in `ownership_cache` keyed by `(head_sha, base_ref, codeowners_oid)`.

| Input | Source | Failure → |
|---|---|---|
| Changed files | GraphQL `files(first:100)` paginated; cap 3000 (GitHub's own limit) | `truncated=true` → confirmation (§7.1) |
| CODEOWNERS | `repository.object(expression: "<base>:.github/CODEOWNERS")`, then `CODEOWNERS`, then `docs/CODEOWNERS`, aliased into one query. The first hit wins (GitHub order). | not found → no CODEOWNERS (user must pick teams) |
| Code-owner requirement | REST `GET /repos/{o}/{r}/rules/branches/{base}`: `pull_request.parameters.require_code_owner_review`. This returns the rules that **actually apply** to the branch, including org rulesets, and replaces the skill's hand-written ruleset matching. | error → `Unknown` |
| Classic branch protection | REST `GET /repos/{o}/{r}/branches/{base}/protection/required_pull_request_reviews` | **403/404 for non-admins is normal** → treat as `Unknown`, not `False` |
| Effective approvals | `latestOpinionatedReviews` (fragment) | — |
| Team membership | REST `GET /orgs/{org}/teams/{slug}/members` (paginated), cached 24 h | 403 → attribution unknown for that team |

`codeowners_required` = `True` if either source says true. It is `False` if `rules/branches` says false **and** protection is known to be false or absent (404 when the caller is an admin). Otherwise it is `Unknown`.

## 7. Slack review-request workflow (reimplementation of `request-slack-review`)

This is the core feature. It ports the skill's scripts (`resolve-pr.py`, `resolve-owners.py`, `resolve-codeowners.py`, `lookup-channel.py`, `upsert-mapping.py`, `build-review-message.py`) to `pr-nexus-core::review`. **Port the skill's unit tests (`test_helpers.py`) as Rust golden tests first** (M3/M4), then apply the Δ fixes marked below, each with its own test.

### 7.1 Readiness gate

Computed from synced fields. Re-computed in the pre-send refresh.

| Condition | Result | Reason text (shown verbatim) |
|---|---|---|
| state Closed/Merged | Blocked | `PR is closed` / `PR is merged` |
| isDraft | Blocked | `PR is a draft` |
| mergeStateStatus DIRTY or mergeable CONFLICTING | Blocked | `PR has merge conflicts` |
| mergeStateStatus BEHIND | Blocked | `PR branch is behind the base branch` |
| any check fail/cancel/timeout/action_required | Blocked | `check <name> is <bucket>` |
| any check pending/queued/in_progress | Blocked | `check <name> is pending` |
| a human's latest opinionated review is CHANGES_REQUESTED | Blocked | `human reviewer <login> requested changes` |
| mergeable UNKNOWN, merge state UNKNOWN/HAS_HOOKS/UNSTABLE | Unknown | `GitHub mergeability is unknown` … |
| mergeStateStatus BLOCKED and only reviews are missing (definition below) | **Ready** (intentional deviation, see below) | note: `GitHub reports BLOCKED: waiting for reviews` |
| mergeStateStatus BLOCKED otherwise | Unknown | `GitHub reports BLOCKED for a reason other than missing reviews` |
| file list truncated (≥3000) | Unknown | `PR file list may be truncated (≥3000 files); confirm additional owners` |
| evidence fetch failed | Unknown | the provider error |

- **Blocked** PRs: the Slack action button (§7.7) is disabled and the tooltip lists the reasons. Blocked PRs cannot be overridden in v1. The skill says "never override", and a red CI is a bad reason to page a team.
- **Unknown** PRs: the preview shows the reasons and requires an explicit "I confirm only reviews are missing" checkbox. The confirmation is scoped to `(head_sha, reasons)`. If either changes, the confirmation is invalidated.
- **Δ BLOCKED: intentional deviation from the skill (decided by the owner).** The skill marks every `BLOCKED` PR as Unknown. Nearly every PR that waits for review reports `BLOCKED`, so the skill asks for confirmation on almost every request. PR-Nexus treats a `BLOCKED` PR as **Ready** when only reviews are missing:
  - `reviewDecision = REVIEW_REQUIRED` (or null with code-owner review pending);
  - `mergeable = MERGEABLE`;
  - the check rollup is SUCCESS, with every check passing, neutral, or skipped;
  - no blocking row above applies (not a draft, not behind, no changes requested).

  The PR detail and preview show a note: "GitHub BLOCKED: waiting for reviews". Any other `BLOCKED` stays Unknown. Other GitHub requirements (e.g. signed commits, deployments) can still hide behind `BLOCKED`. The owner accepted that risk: the worst case is a review request that arrives slightly early. Golden tests must cover both branches.
- **Check buckets:** SUCCESS, NEUTRAL, and SKIPPED pass. Only a check rollup with real items counts. "No checks at all" is Unknown (`no CI checks reported`).

### 7.2 Ownership: CODEOWNERS + branch rules, never requested reviewers

`requested_reviewers` are shown in the UI and **never** used as owners. Owners come only from CODEOWNERS matched against changed files, plus the code-owner requirement from rules/protection.

**CODEOWNERS parsing** follows GitHub semantics. The **last matching line wins**.

Δ fixes vs the skill (each one gets a test):
1. **Keep the team org.** `@acme/team-a` → `TeamRef{org:"acme", slug:"team-a"}`. The skill drops the org, which breaks cross-org teams and the mapping key.
2. **Anchoring.** A pattern with a slash at the start or in the middle is anchored at the repo root. A pattern without a slash matches at any depth. `docs/*` matches `docs/a.md` but **not** `docs/sub/a.md`. The skill matched nested files. `**` follows gitignore rules. A trailing `/` matches the whole directory.
3. **Empty owner list resets.** A line with a pattern and no owners un-owns the matching paths. The skill ignored such lines.
4. **Inline comments.** Text after ` #` is a comment. Escaped `\#` stays literal.
5. User owners (`@login`) and emails are parsed and kept in `users_ignored_for_slack`. They are never mapped to channels.

**Partition** (same as the skill):
- `exclusive` = teams that are the *sole team owner* of at least one changed path.
- `coverage` = exclusive teams, plus every team owning a path that no exclusive team covers.
- `optional` = all other team owners. They co-own paths that are already covered.

**Approval attribution** (removes teams that have already approved).
- Count only the current effective `APPROVED` reviews from `latestOpinionatedReviews`, where the author is a `User` (not a Bot) and not the PR author.
- An approval is attributed to a team when the review carries `onBehalfOf` for that team (positive evidence), **or** the reviewer is a verified member of the team (via `team_members_cache`).
- A verified member of several candidate teams satisfies each of them.
- If membership cannot be fetched, that team goes into `attribution_unknown`. That means confirmation is needed, not that the team has not approved.

**`decide_owners`.** This is a pure function: `(PullRequest, Readiness, Ownership, Tri, approvals, membership, user_team: Option<TeamRef>) -> OwnerDecision`. It is a straight port of `resolve-owners.py::decide_owners`:
1. `candidates` = `[user_team]` if the user picked a team. Otherwise `coverage ∪ optional`.
2. `approved` = candidates with an attributed approval. `remaining = candidates − approved`.
3. If `codeowners_required ∈ {True, False}` and CODEOWNERS loaded: `mandatory = coverage` and `optional = optional`. Optional teams, or no team at all, set `needs_user_choice`. (False still routes to coverage teams, with a rationale note.)
4. If CODEOWNERS failed to load: `needs_user_choice`, and the user picks teams.
5. If `codeowners_required = Unknown`: coverage becomes `routing_hints` (pre-checked in the picker, but the user must confirm).
6. File list truncated: `needs_user_choice`, readiness is Unknown, and the user is asked about other owners.
7. Remove approved and uncertain teams from mandatory, optional, and routing hints.
8. `no_op` = ready ∧ candidates ≠ ∅ ∧ remaining = ∅ ∧ no attribution unknowns ∧ not truncated. The UI shows "All owning teams approved ✓", and Request is disabled.
9. Blocked → no targets. Unknown readiness or attribution → `confirmation_required`, and targets = remaining after confirmation. Otherwise `post_to = mandatory`.

`OwnerDecision` stores all of these lists plus a `rationale: Vec<String>`. The preview shows the rationale in a collapsible "Why these teams?" block.

### 7.3 Channel lookup

For each target team:
1. Effective mapping (§7.5): match `(org, slug)`. The row carries its `slack_workspace`, which selects the Slack account and token used for every later call for this team.
2. Otherwise match `(org, team_name)` exactly. This covers rows that are filled in only by name.
3. A row with `slack_channel_id`: use it. Call `conversations.info` once per session. If the channel is **archived**, refuse to post and open the picker with a banner: "#x is archived — pick its replacement. It will be saved as your correction." (The skill followed redirect messages by hand. The app does not parse "moved to" messages; the saved correction replaces the old channel.)
4. A row with only `slack_channel_name`: resolve it through the cached `conversations.list` (types public+private, `exclude_archived=true`). The user confirms the match once, and it is saved as a user row with an ID.
5. No row, an empty row, or `disabled=1` → the **channel picker**. It is **scoped per workspace**: a workspace switcher (one tab per connected workspace) defaults to the row's workspace, else the last workspace used. It shows a searchable list from `slack_channels_cache` for that workspace only, plus a "paste channel link/ID" fallback (the workspace is detected from the link's domain). The choice is saved as a user correction unless "just this once" is ticked.
6. Never invent a channel ID. Never post to `@channel`/`@here`.

Channel IDs are only unique within a workspace. Every lookup, dedup, post, and cache key is therefore `(workspace, channel_id)`, never `channel_id` alone.

The mapping **starts empty**: no company data ships with the app or the repo. Most teams get a channel on first use through the picker, or in bulk through CSV import (§7.5). The picker and correction flow is therefore the main path, not an edge case.

### 7.4 Review-request state: source of truth, discovery, lifecycle

The app must always know whether a PR already has a review request. **`review_requests` is the source of truth.** `review_messages` is only the message log.

**How rows get created**
- **`source='app'`.** A successful top-level request post inserts one `active` row per (PR, workspace, channel), pointing to the posted message's `ts` (`root_ts`) and permalink.
  - A bump updates the matching active rows: `last_bumped_at = now`, `bump_count += 1`. It never creates a row.
  - If the bump fell back to a new top-level message (§7.9, >3 roots), the old rows are closed as `superseded` and new active rows point to the new message.
- **`source='discovered_via_search'`.** Requests posted outside the app (by hand, or by the skill) are found with `search.messages` and stored the same way.
  - When it runs: in the background, once per open authored PR that has no active request, per connected workspace, at most every 24 h (`discovery_runs`), throttled to 10 calls/min per workspace (`search.messages` is Tier 2). It also runs right before opening the preview for any selected PR with no active row (bypassing the 24 h cooldown), and when the user clicks "Search Slack".
  - Background query (the target channels are not known yet): `"github.com/ORG/REPO/pull/N"` across the workspace. Pre-preview query, once channels are resolved, uses the skill's exact template: `in:<#CHANNEL_ID> "github.com/ORG/REPO/pull/N"`.
  - Match hits whose text contains `https://github.com/ORG/REPO/pull/N` followed by a non-digit or end of text (`/pull/4` must not match `/pull/42`). Keep only hits in channels that some mapping row points to, or that the user picks in the preview.
  - Per (PR, workspace, channel), the oldest matching top-level message is the root: the hit's `thread_ts` if it is a reply, else its `ts`. `requested_at` = root time. Later matching messages in the same thread count as bumps (`last_bumped_at`, `bump_count`).
  - A hit whose root matches a **closed** row is ignored. The `UNIQUE` key prevents resurrection, so a dismissed request stays dismissed.
  - If `search:read` is missing, discovery is skipped and the PR shows "Slack history: app only".

**When a request stops being active (closed).** This decides whether the action is Request or Bump.

| Event | Effect |
|---|---|
| The PR is merged or closed (seen by sync) | all its active rows → `pr_merged` / `pr_closed` |
| The user clicks **Dismiss request** (PR detail, per channel) | that row → `dismissed`. The next action for that channel is a new Request. |
| A new top-level message is posted to the same (workspace, channel) for this PR (user chose "new top-level message", or the >3-roots fallback) | old row → `superseded` |
| The channel is archived (seen on `conversations.info`) | row → `channel_archived`. The next action is a Request to the replacement channel. |
| All owning teams approved | **Not closed.** The action is disabled ("All owning teams approved ✓"). |
| New commits after approval, or the approval is dismissed | **Not closed.** The request stays active, so the action is **Bump**: the re-review belongs in the same thread. |
| The PR is reopened | Closed rows stay closed. The next action is a Request. |

There is no age-based expiry. An old request is still the right thread to bump; the label shows its age.

**What the UI shows.** For each PR, the newest active row per channel gives: "requested 3d ago · bumped 1d ago (2×) in #team-a-reviews · Open in Slack". The list shows the compact indicator `Requested 2d ago · bumped 5h ago ↗` (§8.2).
### 7.5 Mapping store: imported + user layers, CSV import/export

- The mapping starts **empty**. It is filled by CSV import, by the channel picker, and by corrections in the preview. Nothing is bundled or committed.
- Two layers in `team_channel_mappings`:
  - `imported`: rows from a CSV import in `replace` mode. A new `replace` import swaps them out wholesale.
  - `user`: corrections made in the app, or a CSV imported in `merge` mode. A `replace` import never touches them.
- **Precedence: a user row beats an imported row for the same `(org, slug)`, whatever the workspace.** One team maps to one channel in one workspace. A user row with `disabled=1` is a tombstone that hides the imported row. Lookup is one SQL query:
  `SELECT … ORDER BY layer='user' DESC LIMIT 1` on slug, then on name.
- **Δ vs the skill:** in the skill, `~/.slack-team-channel-mapping.csv` *replaces the whole bundled file* (`home_override_replaces_bundled`), so a sparse home file hides every bundled row. Row-level layering fixes that: a re-import of the team's shared CSV keeps your corrections.
- **Δ:** the skill's `upsert-mapping.py` writes CSV columns in a nondeterministic order (it builds them from a Python `set`). PR-Nexus always exports this fixed header:
  `github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id`
  plus an optional 6th column, `slack_workspace` (a workspace domain). The skill's 5-column CSV imports as is. On import, rows without a workspace get the **target workspace the user picks in the import dialog** (required when any row lacks one; the choice lists connected workspaces). A domain with no connected Slack account still imports. That row shows "workspace not connected" and is skipped when sending.
  The export quotes with RFC 4180 rules and sorts rows by `(org, slug)`, so exports are diff-friendly.
- Import modes:
  - `replace`: delete imported rows, then insert.
  - `merge`: upsert into user rows; empty cells do not overwrite non-empty ones.
  - A dry-run diff is shown before applying: added, changed, unchanged, and rows that the user layer shadows.
  - Unknown columns are an error. Missing optional columns default to empty.
- Export modes: `effective` (merged view), `user` (corrections only, for backup), and `imported`.
- **No company data in the repo.** Test fixtures use made-up orgs, teams, and channels (`acme`, `team-a`, `#team-reviews`). The CSV parser is tested on those fixtures (header, column count, unique `(org, slug)`, `C…`/`G…` ID shape).
- The settings UI has a mappings table with filters (org, empty channel, overridden) and inline edits that write user rows. It has a "Reset to imported" action per row, which deletes the user row.

### 7.6 Message text: byte-exact port of `build-review-message.py` (MUST)

`pr-nexus-core::review::message::build(mode, items: &[(title, url)], requester: Option<&str>, extra: Option<&str>) -> String`. Rules:
- Titles and URLs are `trim()`med. An empty title is an error. A URL must start with `http`.
- Lines are joined with `\n`. Then `rstrip()` all trailing whitespace and append exactly one `\n`.
- `who` = `" (" + requester + ")"` if a requester is set, else `""`.
- `extra` (trimmed) is appended after one blank line.
- A single item uses the single-PR form. Two or more items use the batch form.

Single request:
```text
Hello team
Could someone please review {title}?{who}
{url}
Thanks!
```
Single remind. **The requester is never included.** The skill computes `who` and then drops it. Keep that behavior (default D6).
```text
Hello, I'm still looking for a review on {title} when you have a moment :pray:
{url}
Thanks!
```
Batch request:
```text
Hello team
Could someone please review these pull requests?{who}
- {title1}
{url1}
- {title2}
{url2}
Thanks!
```
Batch remind (no requester):
```text
Hello, I'm still looking for a review on these pull requests when you have a moment :pray:
- {title1}
{url1}
…
Thanks!
```

- The bare URL always sits on its own line. Never use `<url|title>`, Markdown links, or code formatting.
- Golden tests: generate fixtures by running the Python script (`uv run scripts/build-review-message.py …`) for about 12 cases: single/batch × request/remind × requester/extra, unicode, titles with `&<>`, and whitespace. Commit the expected outputs under `crates/pr-nexus-core/tests/fixtures/messages/`. Rust output must equal them byte for byte.
- **Wire escaping is separate from the text.** The Slack provider escapes `&`→`&amp;`, `<`→`&lt;`, `>`→`&gt;` in the outgoing `text`, which Slack requires for literal characters. The rendered message is identical to the script's text. The preview shows the unescaped text. The `review_messages.text` column stores the unescaped text.
- `settings.review.requester` defaults to empty, so no requester is added, matching the skill's default.

### 7.7 One contextual action: Request review or Bump request

There is never a separate Request button and Bump button. **In the list**, rows have no action buttons; the floating bar's single **Request / bump on Slack** action applies to the selection (§7.9), and rows only show a request indicator with a link to the original message (§8.2). **In the PR detail**, one contextual button covers the single-PR case. Its label comes from `SlackAction` (§4.1):
- **Request review**: the PR has no active `review_requests` row.
- **Bump request**, next to `requested 2d ago · bumped 5h ago`: the PR has at least one active row.
- **Disabled** with a tooltip: Blocked readiness, all owners approved, not authored by you (D4), or no Slack workspace connected. In bulk, such PRs are not refused at selection time; the preview lists them as skipped.

The preview decides per channel. A PR with an active request in one channel and a newly needed owner team in another channel gets a bump in the first and a request in the second.

| | Request (per channel) | Bump (per channel) |
|---|---|---|
| When | no active row for this (PR, workspace, channel), or the user picks "new top-level message instead" in the preview | an active row exists for this (PR, workspace, channel) |
| Text | request template (§7.6) | remind template |
| Slack call | `chat.postMessage(channel, text, unfurl_links=true)` | `chat.postMessage(channel, text, thread_ts=<root_ts>, reply_broadcast=true)`: posted in the thread **and** in the channel, same as the skill |
| Stored | `review_messages(kind='request')` + new active `review_requests` rows (`source='app'`) | `review_messages(kind='bump', thread_root_ts=<root>)` + `last_bumped_at`, `bump_count` on the active rows |
| Guard | — | Warn (not block) if the last request or bump in this channel is less than `review.min_bump_interval_hours` old (default 24, default D12). |

Both kinds open the same **preview modal** (wireframe and mockup in §8.2):
- one card per `workspace · #channel`, with a **Change channel** dropdown (the picker) and a **Save as mapping correction** checkbox (checked by default; unchecked = only for this message);
- each card is labeled **Request** or **Bump**. A request renders as a new message. A bump renders as a **thread reply under the existing message** (root excerpt plus "Open thread ↗");
- a Slack-style rendering of the exact text (§7.6), unescaped;
- inline warnings for PRs skipped for that channel (`acme/web#91 skipped: CI failing`) and approved teams skipped (with the reviewer's login);
- gate notes: Unknown reasons with the confirm checkbox;
- the GitHub option "Also request <team> as reviewer on GitHub" (default off, D7);
- footer: **Cancel** and primary **Send N messages**.

Changing a channel saves a **user correction** for that team (`layer='user'`) when "Save as mapping correction" is checked.

There is no Slack draft step (Δ). The skill posts drafts first because an agent should not post unseen text. The in-app preview with an explicit Send button serves that purpose, with less friction (default D13).

### 7.8 Refresh-before-write (MUST)

When the user clicks Send, for each affected PR:
1. Re-sync the PR, and its ownership inputs if `head_sha` changed.
2. Recompute readiness and `decide_owners`.
3. Compare with the decision shown in the preview.
   - Same → send.
   - The head changed, new blockers appeared, targets changed, or unknown reasons changed → **abort that channel's message** and re-open the preview with a diff ("PR #42 got a new commit; team X approved meanwhile"). Teams that are now approved simply drop out, and this does not need re-confirmation.
4. Reads and writes are not atomic. This narrows the race window but does not close it. That is acceptable.

### 7.9 Bulk (list view)

- Select PRs with checkboxes (per row, with select-all per project section). A floating bulk bar appears at the bottom: `N selected · [Request / bump on Slack] · [Clear]`. There is **one** bulk action. The planner sorts each PR into a request or a bump per channel, using `review_requests`.
- Pipeline:
  1. gate each PR, dropping Blocked ones with their reason;
  2. run `decide_owners` per PR;
  3. resolve channels;
  4. **group by `(workspace, channel)`: one message per channel lists every selected PR routed to it**. A batch can span several workspaces;
  5. show the preview, with one card per channel. Inside a card, PRs are split into **Requests** (new top-level message) and **Bumps** (thread replies), so a card can hold one of each;
  6. send sequentially;
  7. show per-PR results.
- **Why one message per channel**, not one per PR or per team: this is the skill's hard rule ("never one message per PR link"). It is also kinder to a team channel and keeps Slack writes well below rate limits. Teams that share a channel share one message.
- **Bulk bump grouping.** For each channel, collect the prior thread root of each selected PR.
  - All share one root → a single bump reply in that thread (`reply_broadcast=true`) covering all of them.
  - ≤3 distinct roots → one bump reply per root, each listing only its own PRs. (Δ: the skill falls back to a new top-level message as soon as the roots differ. That loses the thread context the bump exists for.)
  - >3 roots → one new **top-level** message using the remind template; its PRs' old rows become `superseded`. A mix of requested and never-requested PRs in one channel → bump replies (rules above) for the requested ones, plus one new request message for the others. The preview shows this choice explicitly, and the user can switch to "one top-level request for all" (default D5).
- Each Send creates a `batch_id`. Results are recorded in `send_attempts`.
- **Result report** (dialog, then a toast): a table with PR × (channel, outcome `sent|skipped|failed`, detail, permalink). Skipped reasons include: blocked: …, all teams approved, no channel chosen, head changed. A failed message (e.g. `not_in_channel`, `channel_not_found`, `RateLimited`) has a **Retry failed** button that runs the gate again.
- A mid-batch failure does not roll back messages already sent. Each channel is independent.

### 7.10 Chat context panel (scoped read)

The PR detail has a right-hand panel titled "Slack". It shows **only** threads PR-Nexus knows about for this PR: the roots of its `review_requests` rows (active first, then closed, labeled with the close reason) and their `review_messages`. For each thread root, it calls `conversations.replies(channel, ts, limit=50)` and caches the result in `chat_thread_cache` for 5 min. It renders replies with the author's name (`users.info`, cached), time, and text, and links to the permalink.
- There is no channel browsing, no history crawling, and no posting from the panel in v1. (Posting is only through the Slack preview.)
- Optional "Mentions" (off by default): `search.messages "github.com/ORG/REPO/pull/N"` across the workspace, limited to 20 results, to find discussions in other channels. It is one call per click, not polled.

### 7.11 Slack auth, methods, scopes, rate limits

**Auth:** a **user token** (`xoxp-…`) from a Slack app **the user creates in their own workspace**, using the manifest below. Messages are posted *as the user*, which matches the skill (it posts via the user's MCP session) and is what reviewers expect.
- Onboarding is done **once per workspace**. The user creates the app from the manifest in that workspace, installs it, copies the user OAuth token, and pastes it into PR-Nexus. PR-Nexus checks the token with `auth.test`, which returns `team_id`, `team` and `url` (the workspace domain comes from `url`). It then creates the Slack account row and stores the token in the keychain under that workspace (§9).
- Separate workspaces (not one Enterprise Grid org) each need their own internal app. Do not turn on public distribution to share a single app: that could move it into the "distributed app" rate-limit category.
- There is no PR-Nexus-hosted OAuth: there is no server, and the redirect URI would need one.

**Rate-limit note (with source).** Since 2025-05-29, Slack applies stricter limits to **commercially distributed, non-Marketplace apps**: `conversations.history` and `conversations.replies` drop to 1 request/min and 15 objects per call. **Internal, customer-built apps are exempt** and keep Tier 3 (~50+/min).
- Sources: https://docs.slack.dev/changelog/2025/05/29/rate-limit-changes-for-non-marketplace-apps/ and https://docs.slack.dev/changelog/2025/06/03/rate-limits-clarity
- So PR-Nexus **must not** be distributed as a shared Slack app. Each user creates their own internal app. This is also why one central "PR-Nexus Slack app" is not offered.
- The client still honours `429` + `Retry-After`. It limits itself to 1 write/sec per channel and to 1 parallel request per method.

| Method | Tier | Use | Scope (user) |
|---|---|---|---|
| `auth.test` | — | validate token, get user and team ids | — |
| `chat.postMessage` | special (~1/s/channel) | request, bump | `chat:write` |
| `chat.getPermalink` | T4 | permalink after post | — |
| `conversations.list` | T2 | channel picker cache (refreshed daily or on demand) | `channels:read`, `groups:read` |
| `conversations.info` | T3 | archived check, name | `channels:read`, `groups:read` |
| `conversations.replies` | T3 (internal app) | context panel | `channels:history`, `groups:history` |
| `search.messages` | T2 (legacy) | review-request discovery (§7.4), Mentions | `search:read` |
| `users.info` | T4 | author names in panel | `users:read` |

**App manifest** (shipped as `docs/slack-app-manifest.yaml`; the onboarding copies it to the clipboard):

```yaml
display_information:
  name: PR-Nexus (personal)
  description: Posts code-review requests you approve in PR-Nexus.
oauth_config:
  scopes:
    user: [chat:write, channels:read, groups:read, channels:history, groups:history, search:read, users:read]
settings:
  org_deploy_enabled: false
  socket_mode_enabled: false
  token_rotation_enabled: false
```

**Multi-workspace (MUST, v1): any number of workspaces.**
- The user picks which workspaces to use by connecting them in onboarding or Settings → Accounts; no workspace is preconfigured. There is one Slack account per workspace (`accounts.slack_workspace` UNIQUE), each with its own keychain token.
- The `ChatProvider` registry is keyed by workspace. Every write and read picks the provider from the mapping row's `slack_workspace`.
- Mapping rows carry `slack_workspace`. The channel picker and `slack_channels_cache` are scoped per workspace. Discovery search runs only in the target channel's workspace.
- The preview card shows `workspace · #channel` (e.g. `acme · #team-reviews`). Bulk grouping is by `(workspace, channel_id)`, so a batch can post to several workspaces in one Send, each with its own token.
- A workspace that is missing or unauthorized blocks only its own channels. The other workspaces still send, and the result report says "re-auth <workspace>".
- There is no default workspace setting. CSV rows without a workspace get the one picked in the import dialog (§7.5); the picker opens on the row's workspace, else the last one used.

### 7.12 Config: everything org-specific is user data, not code

Nothing org-specific is hard-coded or shipped: GitHub orgs, Slack workspaces, teams, and channels are chosen by the user in onboarding and Settings, and stored in SQLite. The repo contains no company names or company data; examples and fixtures use neutral placeholders (`acme`, `team-a`, `#team-reviews`). The rule lives in AGENTS.md ("no org/workspace/channel literals in code, fixtures use placeholders") and is enforced in review.

| Key (`settings`) | Default | Example user value |
|---|---|---|
| `github.tracked_orgs` | `[]`: none; onboarding asks the user to pick | `["acme","acme-labs"]` |
| `github.include_personal_repos` | `false` | false |
| `github.codeowners_paths` | `[".github/CODEOWNERS","CODEOWNERS","docs/CODEOWNERS"]` | same |
| `github.files_cap` | 3000 | same |
| `sync.focused_interval_s` / `unfocused_interval_s` | 180 / 900 | same |
| `sync.keep_closed_days` | 7 | same |
| `slack.workspaces` | none: derived from connected Slack accounts | `acme`, `acme-labs` |
| `review.requester` | `""` | `""` |
| `review.min_bump_interval_hours` | 24 | 24 |
| `review.blocked_review_required_is_ready` | `true` (§7.1 Δ, owner decision) | true |
| `review.discovery_search` | `true` (if `search:read` is granted) | true |
| `review.bulk_bump_max_threads` | 3 | 3 |
| `review.also_request_github_team` | `false` (checkbox per preview) | false |
| `projects.ticket_key_regex` | `([A-Z][A-Z0-9]+-\d+)` | e.g. restrict to the user's Jira keys |
| mapping | empty; filled by CSV import, picker, corrections | — |

## 8. Projects & UI

### 8.1 Projects and auto-link rules

- A project is a local, user-defined group. It is the primary navigation; orgs are only a filter.
- Links live in `pr_projects` (many-to-many). They are evaluated on every sync for new or changed PRs:
  1. Links with `source='manual'` or `'excluded'` are never touched by rules.
  2. For each active rule, ordered by `priority` desc, a match inserts `source='rule'`. This is skipped if an `excluded` row exists for the same (PR, project).
  3. Rule links whose rule no longer matches are removed. Manual links are kept.
- Rule kinds:
  - `TicketKeyInBranch`: a key extracted from `head_ref` (and title) with `projects.ticket_key_regex`, compared case-insensitively to the pattern (e.g. `PROJ-123` or `PROJ-*`);
  - `BranchRegex`;
  - `Label` (exact);
  - `Repo` (`owner/name` glob);
  - `TitleRegex`.
- Regexes are compiled once, with the `regex` crate (no backtracking, so they are safe against untrusted titles).
- Unlinking a rule-linked PR in the UI writes `excluded`.
- PRs with no project go to an **Unassigned** section, pinned last.

### 8.2 Views (wireframes)

**Visual style.** Dark, Linear-like, dense:
- compact rows (about 44 px, two lines: title, then `owner/repo · CI`);
- collapsible project sections with a count and a select-all checkbox;
- a row checkbox on every PR;
- a **floating bulk action bar** at the bottom center that appears only while something is selected;
- muted text, one accent color (violet) for the primary action and the selection, status pills for review decision.

Approved mockups (AI-generated; style references, not pixel specs):
- [`docs/ui/list-view.jpg`](ui/list-view.jpg): list, filter menu, sync footer, bulk bar. Known differences, where the spec wins:
  - the **per-row Request/Bump buttons are superseded**: rows have no action buttons, only the request indicator below;
  - it omits the unresolved-thread count, **which must still be shown** (💬 n);
  - the filter menu anchors **under the funnel button** (in the mockup it floats near the `⋯` button).
- [`docs/ui/slack-preview.jpg`](ui/slack-preview.jpg): the Slack preview modal. The list behind it is filler. The rendered message is illustrative: the real text is the §7.6 template, byte-exact, and the sender shown is **you** (messages are posted with your user token), not an "APP" bot.

**Layout.**
- **Sidebar** (navigation only, no persistent filters): My PRs, To review, Projects, Settings. There is no org checkbox list here; orgs are a filter in the menu.
- **Sidebar footer:** sync status plus a refresh button: `● Synced · 2m ago  ⟳`, `◌ Syncing…`, `▲ Rate limited · resets 14:32  ⟳`, or `▲ Sync failed · <reason>  ⟳` (§6). Clicking the text shows per-account detail.
- **Header:** view title and subtitle (`8 open PRs across 3 projects`), the **search bar**, a single small **filter icon button** (funnel), and `⋯`.

**List (home)**:

```
┌──────────────────┬──────────────────────────────────────────────────────────────────────────┐
│ ◆ PR-Nexus       │ Pull Requests                        [🔍 Search projects, PRs… ] [⏷] [⋯] │
│                  │ 8 open PRs across 3 projects                                             │
│ ⌂ My PRs       ● │ ┌ ▾ ☐ Auth V2 (3) ─────────────────────────────────────────────────────┐ │
│ ◷ To review      │ │ ☑ #142 Migrate login to new auth  ✓Approved 💬0/2  Requested 2d ↗    │ │
│ ▤ Projects       │ │   acme/webapp · ● Passing                           bumped 5h ago    │ │
│ ⚙ Settings       │ │ ☑ #138 Add passkeys support       ● Review  💬1/4                    │ │
│                  │ │   acme/auth-service · ● Passing                                      │ │
│                  │ │ ☐ #135 Refresh token rotation     ✕ Changes 💬3/3                    │ │
│                  │ │   acme/auth-service · ● Failing                                      │ │
│                  │ └──────────────────────────────────────────────────────────────────────┘ │
│                  │ ▸ ☐ Billing revamp (2) · collapsed                                       │
│                  │ ┌ ▾ ☐ Unassigned (1) ──────────────────────────────────────────────────┐ │
│                  │ │ ☐ #23  Fix tax calculation        ● Review  💬0/0  Requested 5d ↗    │ │
│                  │ │   acme/billing · ● Passing                                           │ │
│                  │ └──────────────────────────────────────────────────────────────────────┘ │
│ ● Synced · 2m  ⟳ │      ┌ 2 selected │ [Request / bump on Slack] │ [✕ Clear] ┐              │
└──────────────────┴──────────────────────────────────────────────────────────────────────────┘
 💬 unresolved/total threads · ↗ opens the original Slack request/thread · no per-row action buttons
```

**Rows have no action buttons.** A row is selected only with its checkbox (or `x`). The **only** Slack action in the list is the floating bar's **Request / bump on Slack**, which decides request vs bump per PR and per channel from `review_requests` (§7.7, §7.9).

Row content, each part toggled under **Show**: number and title; `owner/repo`; CI; review-decision pill; 💬 unresolved/total threads (default on); owning teams; last activity; project badges (`+1` when in several projects); the **Slack request indicator**.

**Slack request indicator** (from the newest active `review_requests` row):
- `Requested 2d ago`, plus `· bumped 5h ago` when `last_bumped_at` is set. Tooltip: `workspace · #channel`, bump count, and source (app or discovered). With several active channels: `Requested 2d ago · 2 channels`, with a hover list.
- A small **link icon ↗** opens the **original request message/thread**. It tries the Slack app first with `slack://channel?team=<team_id>&id=<channel_id>&message=<root_ts>`, where `team_id` comes from the Slack account's `external_id`. If no handler is registered (`tauri-plugin-opener` errors), it opens the stored https `permalink` in the browser. Verify the deep-link format in M10; if Slack ignores `message`, use the permalink directly, since the desktop app intercepts Slack https links when installed.
- Rows without an active request show nothing by default. Show → "Not requested label" adds a subtle grey `Not requested`.

**Filter menu** (funnel button; opens a menu like Linear/Cursor, with submenus on hover/→):

```
 [⏷]
 ┌──────────────────────────────┐   ┌─────────────────┐
 │ Grouping          Project  › │──▶│ ✓ Project       │
 │ Ordering          Updated  › │   │   Repository    │
 │ Show                       › │   │   Org           │
 ├──────────────────────────────┤   │   Status        │
 │ Filters               Reset  │   │   Updated       │
 │ Org                        › │   └─────────────────┘
 │ Status / draft             › │
 │ Review decision            › │
 │ CI                         › │
 │ Slack request              › │
 │ Archived                   ☐ │
 └──────────────────────────────┘
```

- **Grouping:** Project (default), Repository, Org, Status (readiness), Updated (Today / This week / Older). Many-to-many PRs appear in each project group.
- **Ordering:** Updated (default, desc), Created, Last Slack request, Readiness, Number; each with asc/desc.
- **Show:** toggles for the row parts listed above.
- **Filters:** multi-select submenus, each listing values with counts:
  - Org: tracked orgs only;
  - Status / draft: Ready, Unknown, Blocked, Draft;
  - Review decision: Approved, Changes requested, Review required, None;
  - CI: Passing, Failing, Pending, None;
  - Slack request: None, Requested, Bumped, Bump due (last request or bump older than `review.min_bump_interval_hours`);
  - Archived: show PRs in archived projects and hidden closed PRs.
  **Reset** clears filters only, not grouping, ordering, or Show.
- The funnel shows a dot when any filter is active. There is no other persistent filter UI.
- **Persistence:** grouping, ordering, Show, and filters are saved **per view** in `view_state` (`mine`, `to_review`, `project:<id>`) and restored on launch.

**Search bar:** always visible. Filters the current view as you type (150 ms debounce), case- and accent-insensitive, whitespace-separated terms ANDed. It matches project name, PR title, PR body (`bodyText`), `owner/repo`, and number (`#142` or `142`). It runs in core over the cached rows (a few hundred PRs; no FTS needed). It combines with the menu filters. `/` focuses it, and `Esc` clears it.

Keyboard: `j/k` move, `x` toggle select, `r` open the Slack preview for the selection (or the focused row if nothing is selected), `s` open the focused row's Slack request, `o` open in browser, `f` open the filter menu.

**PR detail**:

```
┌ acme/repo-a #412 · Add retry budget ────────────────────────── [Open in GitHub] ───┐
│ main ← feat/PROJ-123-retry   by you · updated 2h ago   Projects: [Billing ×] [+]   │
│ CI ✓ success · Mergeable ✓ · Review: REVIEW_REQUIRED · Threads 0/3 unresolved      │
│ Readiness: Ready (note: GitHub BLOCKED, waiting for reviews)                       │
│ Owners: team-a (exclusive) · platform (optional) · approved: —                     │
│ Requested on GitHub (info only): @alice                                            │
│ Slack: requested 3d ago · bumped 1d ago (1×) in acme · #team-a-reviews             │
│        Open in Slack ↗ · Dismiss request                       [Bump request]      │
├───────────────────────────────────────────────┬────────────────────────────────────┤
│ Why these teams? ▸                            │ Slack                              │
│ Files (37) ▸                                  │ #team-a-reviews · request · 3d     │
│                                               │   bob: looking tomorrow            │
└───────────────────────────────────────────────┴────────────────────────────────────┘
```

**Slack preview modal** (approved design, centered; see `docs/ui/slack-preview.jpg`):

```
┌ Preview Slack messages (2 channels, 3 PRs) ───────────────────────────────────────┐
│ Review the messages below before sending to Slack.                                │
│ ┌───────────────────────────────────────────────────────────────────────────────┐ │
│ │ acme · #team-a-reviews                  REQUEST  [Change channel ▾]           │ │
│ │                                                  ☑ Save as mapping correction │ │
│ │  (you)  10:42                                                                 │ │
│ │  Hello team                                                                   │ │
│ │  Could someone please review these pull requests?                             │ │
│ │  - Migrate login to new auth flow                                             │ │
│ │  https://github.com/acme/webapp/pull/142                                      │ │
│ │  - Add passkeys support                                                       │ │
│ │  https://github.com/acme/auth-service/pull/138                                │ │
│ │  Thanks!                                                                      │ │
│ │  Extra text: [                                    ]                           │ │
│ ├───────────────────────────────────────────────────────────────────────────────┤ │
│ │ acme · #team-b-reviews                  BUMP     [Change channel ▾]           │ │
│ │                                                  ☑ Save as mapping correction │ │
│ │  ┊ (you) 3d ago: Hello team … /pull/23                     Open thread ↗      │ │
│ │  └─ (you) 10:42 · reply, also sent to channel                                 │ │
│ │     Hello, I'm still looking for a review on Fix tax calculation edge case    │ │
│ │     when you have a moment :pray:                                             │ │
│ │     https://github.com/acme/billing/pull/23                                   │ │
│ │     Thanks!                                                                   │ │
│ │  ⚠ acme/web#91 skipped: CI failing                                            │ │
│ └───────────────────────────────────────────────────────────────────────────────┘ │
│ ☐ I confirm only reviews are missing for #77 (GitHub mergeability is unknown)     │
│ ☐ Also request teams as reviewers on GitHub                                       │
│                                               [Cancel]  [Send 2 messages ➤]       │
└───────────────────────────────────────────────────────────────────────────────────┘
```

- The title counts channels and distinct PRs. "Send N messages" counts Slack posts: a channel with both requests and bumps counts 2, and a bump split over several threads counts one per thread.
- A card that holds both kinds shows a **Request** section, then a **Bump** section.
- Skipped PRs show as inline yellow warnings in the card of the channel they would have gone to, or in a "Not sent" card if no channel was resolved.

**Projects**: list on the left; on the right, name/color, rules editor (kind, pattern, priority, and a live "matches N open PRs" preview), and linked PRs with their source badges (manual/rule/excluded).

**Settings** tabs:
- Accounts (GitHub account plus the **tracked orgs picker**: checkboxes from `viewer.organizations` and orgs seen in search, and a "personal repos" toggle; Slack with one row per connected workspace, each with its own status, re-auth, remove, and an "Add workspace" button);
- Mappings (table, import/export, dry-run diff);
- Review (the config keys in §7.12);
- Sync;
- About (version, log folder, DB path).

## 9. Auth, secrets & security

**GitHub auth**, in the order offered:
1. **`gh` CLI token** (default if `gh auth status` succeeds). Run `gh auth token --hostname github.com` on demand, with a 5 s timeout, and keep the result in memory only. It is **never stored**. It inherits the user's SSO authorizations, which is usually the least friction in SAML orgs.
2. **Fine-grained or classic PAT**, pasted by the user and stored in the keychain.
   - Classic: `repo`, `read:org`.
   - Fine-grained: per org, read access to Pull requests, Contents (for CODEOWNERS), Metadata, Checks/Commit statuses; Members (org) read for team membership; Pull requests write only for the M10 option.
   - An org may block classic or fine-grained PATs. That is why the `gh` token is the default (D1).
3. **OAuth device flow** with a PR-Nexus OAuth App (client id embedded, no secret). The token is stored in the keychain. An org may need to approve the app.

Detect `X-GitHub-SSO: required; url=…` and show the authorization link.

**Secrets**:
- Use `keyring` with service `dev.pr-nexus` and **one single entry**, `vault`. It holds a JSON map `{ "<kind>:<external_id>": "<token>" }`, e.g. `github:<node id>` and one `slack:<team_id>:<user_id>` per workspace. The app reads it once at startup and keeps it in memory (`secrecy::SecretString`).
- **Why one entry:** builds are unsigned (see the caveat below). After each update, macOS asks once per keychain item whether to allow access. With 1 GitHub account and N Slack workspaces, separate items would mean 1+N prompts per update; one entry means one.
- Never write secrets to SQLite, logs, or the frontend. Commands return only `has_token: bool`.
- In Linux CI there is no secret service, so tests use the keyring `mock` store behind a feature flag.

**Webview hardening**:
- Tauri v2 capabilities give the main window only `core:default`, `opener:allow-open-url` limited to `https://github.com/*` and `https://*.slack.com/*`, and the app's own commands.
- No shell or fs plugins in the webview.
- CSP: `default-src 'self'; img-src 'self' https://avatars.githubusercontent.com data:; connect-src ipc: http://ipc.localhost`. All network access happens in Rust.
- Message text, titles, and Slack replies render as text, never as HTML.

**Logs**:
- Use `tracing` with daily rotating files in the app log dir, kept for 7 days.
- A redaction layer masks `gh[pousr]_…`, `github_pat_…`, `xox[abposr]-…`, and `Authorization` headers.
- Message bodies log at debug level only.

**macOS distribution without an Apple Developer ID (owner decision: v1 is unsigned/ad-hoc).**
- *Gatekeeper.* The DMG is ad-hoc sealed (`codesign --force --deep --sign -`), not notarized. A browser download gets the quarantine flag, and macOS refuses the first launch ("cannot verify the developer" or "damaged"). Workarounds, documented in README and CONTRIBUTING:
  1. **Preferred:** `gh release download vX.Y.Z -R github-christophe-oudar/pr-nexus -p '*.dmg'`. The repo is private, so `gh` is the natural path anyway, and files fetched by `gh`/`curl` carry no quarantine flag.
  2. Browser download: try to open the app once, then System Settings → Privacy & Security → **Open Anyway**. Control-click → Open no longer bypasses this on macOS 15+.
  3. Or `xattr -dr com.apple.quarantine /Applications/PR-Nexus.app`.
- *Keychain re-prompt after every update.* An ad-hoc signature has no stable team identity, so the keychain's "Always Allow" rule is tied to the exact binary hash. Each new version triggers a "PR-Nexus wants to use your confidential information stored in 'dev.pr-nexus'" prompt, which needs the login password and then **Always Allow**. The single `vault` entry above keeps this to **one prompt per update**. The About tab and release notes explain it. Denying the prompt is handled: accounts show "token unavailable — unlock", with a retry button. Nothing is wiped.
- *Future, optional:* Developer ID signing + notarization (Tauri reads `APPLE_*` secrets) would remove both issues. Nothing in v1 depends on it.

## 10. Future (out of v1)

### 10.1 MCP server

- **Shape.** A separate sidecar binary crate, `crates/pr-nexus-mcp`, built on `rmcp` 3.x (the official Rust SDK). The Tauri app ships it as `bundle.externalBin`.
- **Transports.** stdio, so Claude Desktop/Cursor can spawn it, and **Streamable HTTP** bound to `127.0.0.1` with a random bearer token shown in Settings. Not SSE, which the MCP spec deprecated in favour of Streamable HTTP.
- **Data.** It opens the **same SQLite file** in WAL mode, with readers plus one short-lived writer, so it works whether or not the app is open. It reads tokens from the same `vault` keychain entry.
- **Read tools** (enabled by default):
  - `list_prs(filter)`;
  - `get_pr(id)` with readiness, owners, and review-request state;
  - `list_projects`;
  - `get_mapping(team)`;
  - `get_slack_thread(pr)`.
- **Write tools**, **disabled by default**, each toggled separately in Settings:
  - `request_or_bump(pr_ids, dry_run=true)`: same planner as the bulk bar, so request vs bump comes from `review_requests`;
  - `dismiss_request(pr_id, channel)`;
  - `link_pr_to_project`;
  - `upsert_mapping`.
- **Write-tool rules.** Writes go through the exact same core pipeline: gate, refresh-before-write, one message per channel. They are allowed only to channels on a **channel allowlist** (empty by default). `dry_run` defaults to true and returns the preview text. Every call is logged to `send_attempts` with `origin='mcp'`.
- **Prompt-injection safeguards.** PR titles and Slack replies are untrusted content. Tool results wrap them as data. The server never follows instructions from that content, and no tool can post free-form text (only the templates).

### 10.2 Other providers & platforms
- GitLab and Gitea (`VcsProvider`): CODEOWNERS equivalents exist (GitLab `CODEOWNERS` with sections), so `decide_owners` stays as is, and new inputs go through `OwnershipInputs`.
- Discord and Mattermost (`ChatProvider`): bump maps to "reply in thread and also to channel" where supported, otherwise a quote plus link.
- Android/iOS via Tauri mobile. Keep `core` free of desktop-only dependencies. The keychain moves behind a trait.
- Scheduled reminders, e.g. "bump if no review after 48 h", using the same pipeline with a user-approved standing rule.

## 11. CI/CD & release process (modeled on `harnesstap/harnesstap`)

### 11.1 What harnesstap does (inspected read-only, 2026-10-25)

harnesstap is a Bun/TypeScript CLI plus a Tauri v2 desktop app in `apps/desktop`. Its pipeline:

- **`ci.yml`**
  - Triggers: `pull_request`, `push` to main, `merge_group`, a nightly cron `17 6 * * *`, and `workflow_dispatch`. Permissions: `contents: read`.
  - `dorny/paths-filter` in a `detect-changes` job gates the expensive jobs.
  - An aggregator job `validate` with `if: always()` fails if any needed job failed or was cancelled. The ruleset requires this single stable check name.
  - The desktop jobs check `Cargo.lock` with `cargo update --workspace --locked` and run typecheck.
  - A sidecar build runs on a 3-OS matrix. Playwright visual tests run in the `mcr.microsoft.com/playwright:v1.63.0-noble` container. Default runner: `ubuntu-26.04`.
  - **No `cargo fmt`, `clippy` or `cargo test` jobs.**
- **Branch ruleset `main-ci-required`** on the default branch:
  - no deletion and no force-push;
  - required checks (test-core/platforms/plugins/services/cli, desktop-typecheck, validate);
  - **merge queue** (squash, ALLGREEN, max 5 entries);
  - delete branch on merge.
- **Changelog via Changie.**
  - `.changie.yaml` kinds: Breaking→major; Features/Added→minor; Changed/Fixes/Fixed/Docs/Dependencies→patch.
  - Each PR adds a fragment in `.changes/unreleased/`.
  - `AGENTS.md` says: never edit `CHANGELOG.md` by hand.
- **`tag-release.yml`**. Triggered by manual dispatch on main, and by a push to main touching `CHANGELOG.md`. Steps:
  1. `changie batch auto` and `changie merge`;
  2. `scripts/stamp-release-version.sh`, which writes the version into `package.json`, `apps/desktop/package.json`, `tauri.conf.json`, `Cargo.toml`, and `Cargo.lock`;
  3. a lock check;
  4. commit `Release vX` as `github-actions[bot]`, then push the tag;
  5. upload a `tag-release-result` artifact.
- **`release.yml`**
  - Triggers: tag push `v*`, dispatch, and `workflow_run` of "Tag release". The last one is needed because tags pushed with `GITHUB_TOKEN` do not trigger workflows.
  - `concurrency: release`. Steps are idempotent. It verifies the version stamps.
  - It publishes to npm via OIDC and creates a **draft** GitHub release with notes from `.changes/vX.md`.
  - Desktop matrix: macos-latest (arm64), macos-15-intel, windows-latest, windows-11-arm, ubuntu-22.04, ubuntu-22.04-arm. It uses `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2` (workspaces `apps/desktop/src-tauri`), and Bun 1.3.14, and builds with `bun run desktop:build`.
  - **macOS: unsigned**, then an ad-hoc re-seal (`reseal-macos-app.sh`: `codesign --force --deep --sign -`, then rebuild the DMG). No notarization.
  - Linux: deb and rpm, then AppImage.
  - Assets are uploaded with `gh release upload --clobber`. `publish-release` verifies the expected asset count before publishing, because releases are immutable. `--latest` is set only when the tag is the highest version.
  - **No updater** (`plugins: {}`).
- **`desktop-e2e.yml`**: nightly, pinned to `ubuntu-24.04`, because `webkit2gtk-driver` is not available on 26.04.

### 11.2 Copy / change / why

| Item | Decision | Why |
|---|---|---|
| Triggers incl. `merge_group` + nightly cron + dispatch | **Copy** | Merge queue needs `merge_group`; nightly catches toolchain/dep drift. |
| `paths-filter` + aggregator `validate` with `if: always()` | **Copy** | One stable required check name. Docs-only PRs stay cheap. |
| Ruleset: no force-push/deletion, required `validate`, squash merge queue, delete-on-merge | **Copy** (set up by the user in GitHub settings, documented in CONTRIBUTING) | Proven setup. Linear history works well with Changie. |
| `contents: read` default permissions, write only in release jobs | **Copy** | Least privilege. |
| Changie fragments + "never edit CHANGELOG.md" in AGENTS.md | **Copy** (same kinds) | Agents add a fragment per PR and avoid conflicts in CHANGELOG.md. |
| `tag-release.yml` (changie batch → stamp → bot commit → tag) | **Copy, simplified** | The version lives in **one place**: `[workspace.package] version` in the root `Cargo.toml`. `tauri.conf.json` omits `version`, so Tauri falls back to `Cargo.toml`. `app/package.json` is `private` with no meaningful version. The stamp script edits 1 file plus `Cargo.lock` instead of 5. |
| `release.yml` triggered by tag + `workflow_run` + dispatch, `concurrency`, idempotent upload with `--clobber`, asset-count check, `--latest` only for highest | **Copy** | These handle real GitHub edge cases (GITHUB_TOKEN tags, immutable releases, re-runs). |
| npm publish via OIDC | **Drop** | No npm package. |
| 6-target desktop matrix | **Change** | v1 **ships macOS arm64 DMG only**. Linux (ubuntu-24.04) and Windows (windows-latest) **build** in CI (`cargo check` + `tauri build --no-bundle`) but are not attached to releases. Add Intel macOS and Linux/Windows bundles when someone asks. |
| Unsigned + ad-hoc reseal | **Copy (owner decision: no Apple Developer ID)** | The ad-hoc reseal is needed for arm64 Gatekeeper. Consequences and workarounds (Open Anyway / `gh release download`, one keychain re-prompt per update) are in §9. Developer ID signing + notarization is optional future work. The release job keeps a dormant `if: secrets.APPLE_CERTIFICATE != ''` branch so it can be enabled without restructuring. |
| No updater | **Copy for v1 (default D10)** | An auto-updater on a private repo needs authenticated downloads, and unsigned builds re-prompt for the keychain anyway. v1 shows an "update available" banner (GitHub releases API with the user's GitHub token) and the user installs the DMG by hand. `tauri-plugin-updater` + minisign comes later. |
| No fmt/clippy/test jobs | **Change: add them** | Agents write most of the code. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, plus `bun run lint`, `tsc --noEmit`, and `vitest` are the cheapest guardrails. |
| Cargo.lock `--locked` check | **Copy** | Catches lockfile drift. |
| — | **Add** `cargo deny check` (licenses, advisories, bans) + Dependabot (cargo, npm, github-actions; weekly; grouped) | A desktop app that holds tokens needs supply-chain hygiene. |
| — | **Add** CSV parser test on placeholder fixtures | No company data in the repo (§7.5, §7.12). (No org-name grep guard: the AGENTS.md rule plus review cover it.) |
| `ubuntu-26.04` default runner | **Change: pin `ubuntu-24.04`** | Tauri on Linux needs `libwebkit2gtk-4.1-dev`. harnesstap already had to pin 24.04 for webkit2gtk-driver. Use one known-good image. |
| Playwright visual container, desktop-e2e | **Defer** | v1 tests the frontend with vitest + React Testing Library against mocked commands. Add tauri-driver e2e when the UI stabilises (Linux only; macOS has no WebDriver). |
| `rust-toolchain@stable` + `Swatinem/rust-cache@v2` + Bun 1.3.x | **Copy** | Pin the toolchain in `rust-toolchain.toml` (stable, with `rustfmt` and `clippy` components). |

### 11.3 Target workflows

**`.github/workflows/ci.yml`** (`permissions: contents: read`; `concurrency: ci-${{ github.ref }}` with cancel-in-progress for PRs)

| Job | Runner | Steps |
|---|---|---|
| `changes` | ubuntu-24.04 | `dorny/paths-filter`: outputs `rust` (`crates/**`, `app/src-tauri/**`, `Cargo.*`), `web` (`app/**` excl. src-tauri), `ci` (`.github/**`) |
| `rust-lint` | ubuntu-24.04 | apt webkit deps; `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo update --workspace --locked` |
| `rust-test` | ubuntu-24.04, macos-latest | `cargo test --workspace --locked` (keyring mock feature on Linux) |
| `web` | ubuntu-24.04 | `bun install --frozen-lockfile`; `bun run lint`; `bun run typecheck`; `bun run test` |
| `build-check` | macos-latest, ubuntu-24.04, windows-latest | `bun run tauri build --no-bundle` (proves all 3 OSes build; macOS also uploads the `.app` as a 7-day artifact) |
| `deny` | ubuntu-24.04 | `EmbarkStudios/cargo-deny-action` |
| `guards` | ubuntu-24.04 | Changie fragment present unless the PR has the `no-changelog` label |
| `validate` | ubuntu-24.04, `if: always()` | fail if any needed job is `failure` or `cancelled` (skipped is fine) |

**`nightly.yml`**: cron + dispatch. Runs the full CI with `cargo update` (no `--locked`) against the latest deps, plus `cargo deny`. It opens no issues. The status shows in the Actions tab.

**`tag-release.yml`**: `workflow_dispatch` on main.
1. Install Changie.
2. `changie batch auto`, then `changie merge`.
3. `scripts/stamp-release-version.sh <ver>`, which edits `[workspace.package] version` and runs `cargo update --workspace`.
4. Verify `--locked`.
5. Commit `Release vX.Y.Z` as `github-actions[bot]` and push the tag.

**`release.yml`**:
- Triggers: `push: tags: ['v*']`, `workflow_run` of "Tag release" (completed + success), and dispatch with a `tag` input. `concurrency: release`, no cancel.
- Jobs:
  1. `prepare`: resolve the tag, verify that `Cargo.toml` has the same version, create or reuse a **draft** release with notes from `.changes/<ver>.md`.
  2. `macos`: macos-latest, arm64.
     - Run `bun run tauri build --bundles app,dmg`.
     - Run `scripts/reseal-macos-app.sh` (ad-hoc, copied from harnesstap). The Developer ID path stays dormant until `APPLE_*` secrets exist (future).
     - `gh release upload --clobber`.
  3. `publish`: verify that the expected assets exist (`PR-Nexus_<ver>_aarch64.dmg`, plus `.app.tar.gz` and `.sig` once the updater lands), then `gh release edit --draft=false`, adding `--latest` only for the highest semver.

Release runbook (CONTRIBUTING.md):
1. Merge PRs with fragments.
2. Dispatch "Tag release".
3. Watch "Release".
4. Smoke-test the DMG on a clean user account: `gh release download` → first launch → add accounts (GitHub + two test Slack workspaces) → pick orgs → sync → install over the old version and confirm exactly one keychain prompt.

## 12. Milestones (each one is a mergeable PR series with acceptance criteria)

| # | Milestone | Acceptance criteria |
|---|---|---|
| **M0** | Bootstrap + CI | Workspace layout (§3) compiles. Tauri window shows "Hello". `ci.yml` is green with fmt/clippy/test/web/build-check on 3 OSes/deny/guards/validate. AGENTS.md, CONTRIBUTING.md, `.changie.yaml`, Dependabot, `rust-toolchain.toml`. The ruleset is documented. |
| **M1** | Release pipeline (unsigned) | `tag-release.yml` and `release.yml` produce a draft→published GitHub release `v0.1.0` with an ad-hoc-sealed arm64 DMG that launches on a clean Mac (via `gh release download`, or Open Anyway after a browser download; §9). Re-running the release is idempotent. |
| **M2** | Store | Migration 0001. Store actor (write thread + read pool) with WAL. Repository functions per table. Tests for migrations, enum round-trip, cascade deletes, the no-secret-column test, the `review_requests` constraints (one active row per (PR, workspace, channel); `close_reason` set iff closed), and `view_state` round-trip. |
| **M3** | Message builder | 12+ golden fixtures generated from the Python script pass byte for byte. Wire-escaping unit tests. |
| **M4** | CODEOWNERS + owner decision | The skill's `test_helpers.py` cases are ported and pass, and the Δ1–Δ4 tests pass. Readiness table (§7.1) implemented as a pure function with a test per row. `decide_owners` rules 1–9 tested with fixtures. |
| **M5** | Mapping | Empty mapping on first run. CSV import (replace, merge, dry-run diff, target-workspace picker for rows without a workspace; the skill's 5-column CSV accepted) and export (effective/user/imported, fixed header, sorted). Layer precedence and tombstone tests on placeholder fixtures. |
| **M6** | GitHub sync | Accounts (gh token, PAT). Tracked-orgs picker (onboarding + Settings; empty state until orgs are picked). Search buckets, client-side tracked-org filter, nodes refresh, polling scheduler, rate-limit pause. HTTP fixture tests via `wiremock`. Typed errors mapped (401, 403-SSO, 403-secondary, 404). |
| **M7** | List UI | Dark compact style (§8.2). Sidebar navigation with the sync-status footer and refresh button. Grouped, collapsible list with checkbox selection, select-all per section, and no per-row action buttons. Floating bulk bar. Slack request indicator with link icon. Filter menu (Grouping, Ordering, Show, Filters with Reset), persisted per view in `view_state`. Search bar over project name, title, body, repo, and number. Keyboard shortcuts. vitest: selection dedupe across sections, filter and search predicates, view-state restore. |
| **M8** | Projects | CRUD, rules with live preview, manual/rule/excluded semantics tested. Unassigned section. |
| **M9** | Ownership inputs | Files (paginated, capped), CODEOWNERS lookup order, `rules/branches`, protection with 403→Unknown, reviews with onBehalfOf, team members cache. Detail view shows readiness, owners, and the rationale. |
| **M10** | Slack single request/bump | Slack onboarding for **any number of workspaces** (manifest + token + `auth.test` per workspace; workspace-scoped channel cache and picker; mapping rows routed to the right token). Channel cache and picker. Archived refusal. `review_requests` lifecycle: created on request; bump updates; closed on merge/close, dismiss, supersede, or archived channel. Discovery via search (background with throttle and cooldown, plus before the preview) stores `source='discovered_via_search'`. The PR detail's contextual Request review / Bump request button. Preview modal with Request/Bump cards → refresh-before-write → post. Bump in thread with `reply_broadcast`. Request indicator deep link (`slack://` with https permalink fallback). Corrections saved as user rows. Optional GitHub team review request. Tested against `FakeChat`, plus one manual run in a test channel. |
| **M11** | Bulk | Single bulk action **Request / bump on Slack**. Per-channel grouping with request and bump sections per card, bulk bump grouping (§7.9), result report, retry failed. Planner unit tests for mixed cases (requested + never requested in one channel, 1/≤3/>3 roots, mixed workspaces, a closed request → new request). |
| **M12** | Context panel | Threads for known messages, cache, user names, Mentions opt-in. |
| **M13** | Polish | Settings complete, log redaction test, "update available" banner (GitHub releases API with the user's token, manual DMG download), keychain re-prompt explainer, onboarding flow, empty states. |

M0–M1 come first on purpose. Every later PR is then built, linted, and releasable from day one, which matters when coding agents produce most of the commits.

## 13. Testing strategy

- **core**: pure unit tests and golden files (messages, CODEOWNERS, decisions, mapping CSV). `insta` snapshots for `OwnerDecision` JSON. Property tests (`proptest`) for the CODEOWNERS matcher against a small reference implementation of gitignore-style matching.
- **providers**: `wiremock` HTTP fixtures, recorded once from real responses and scrubbed of names and tokens. Each fixture covers one shape: search page, PR fragment, 403 SSO, secondary rate limit, Slack `ratelimited` with `Retry-After`, `not_in_channel`.
- **review requests**: table-driven lifecycle tests in core: sync sees merged/closed → closed; dismiss → next action Request; new commits after approval → still Bump; reopened PR → Request; discovery hit on a closed root → ignored; `/pull/4` vs `/pull/42`; oldest root wins; thread replies count as bumps.
- **app**: command-level tests with `FakeVcs`/`FakeChat` and a temp DB. Frontend tests with vitest + Testing Library against a mocked `invoke`.
- **Manual pre-release checklist**: request and bump in a private test channel; verify link rendering, thread plus channel broadcast, permalink, and that the row's ↗ opens the original message in the Slack app.
- There is no live-API test in CI.

## 14. Open questions: answered (A) and defaults chosen (D)

**Answered by the owner (2026-10-25)**
- **A1. No company names or data in the repo or spec.** GitHub orgs and Slack workspaces are chosen by the user in onboarding and Settings; nothing is hardcoded or defaulted. The team → channel mapping starts empty (no committed seed). Names stay out of code (§7.12).
- **A2. Multi-workspace Slack in v1 (N workspaces):** per-workspace token in the keyring vault, `slack_workspace` on mapping rows, workspace-scoped channel cache and picker, `(workspace, channel)` grouping (§7.11).
- **A3. PR ↔ project is many-to-many** (§4.1, §8.1).
- **A4. GitHub BLOCKED with only reviews missing = Ready.** This is an intentional deviation from the skill (§7.1).
- **A5. No Apple Developer ID.** Unsigned/ad-hoc macOS builds. Gatekeeper workaround and one keychain re-prompt per update are documented (§9). Signing is optional future work (§11.2).

**Defaults chosen (change them by editing this spec)**
- **D1. GitHub auth:** the `gh` CLI token is the default, with PAT and then device flow as fallbacks. SSO errors show the authorization link.
- **D2. Readiness otherwise follows the skill:** `BEHIND`, pending CI, and failed CI block. Blocked cannot be overridden in v1.
- **D3. Ticket regex:** generic `([A-Z][A-Z0-9]+-\d+)` from branch and title. The user narrows it per project rule.
- **D4. "To review" filter ships in v1,** backed by the `review_requested` bucket. The Slack action is disabled for PRs you did not author, and they are skipped in bulk.
- **D5. Bulk bump with different threads:** one reply per thread (≤3 per channel), otherwise one top-level reminder. The preview can switch to a single top-level message.
- **D6. Requester suffix:** none by default (`review.requester = ""`), and never on reminders, as in the skill.
- **D7. GitHub team review request:** offered as a checkbox in the preview, off by default. It is the only GitHub write.
- **D8. `search.messages` (legacy) is used to discover requests posted outside the app** (§7.4). Without `search:read`, only app-posted requests are known.
- **D9. CSV workspace:** the CSV may carry a `slack_workspace` column. Rows without one get the target workspace the user picks in the import dialog.
- **D10. No auto-updater in v1:** an "update available" banner, and manual DMG install.
- **D11. Linux/Windows build in CI only,** with no release assets.
- **D12. Bump interval:** a warning (not a block) below 24 h.
- **D13. No Slack drafts:** the in-app preview plus an explicit Send replaces the skill's draft-first flow.
- **D14. Review-request lifecycle:** a request is closed only by merge/close, explicit dismiss, being superseded by a new top-level message in the same channel, or an archived channel. Approvals and new commits do not close it, and there is no age expiry (§7.4).
- **D15. Discovery cadence:** background search at most once per PR per workspace every 24 h, ≤10 calls/min per workspace, plus a forced check before the preview (§7.4).
- **D16. List actions:** no per-row action buttons; the bulk bar is the only list action, and the PR detail keeps a contextual button (§7.7, §8.2).
