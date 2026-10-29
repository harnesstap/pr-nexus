# PR Nexus — Technical Specification (v1)

Status: v1 spec, 2026-10-25 (open questions answered; see §14) · Owner: Christophe Oudar · Audience: coding agents bootstrapping this repo, and human reviewers.

This file is the source of truth for product behavior. When code and spec disagree, fix one of them in the same PR. Sections marked **MUST** are acceptance-relevant. Everything marked *Future* is out of scope for v1. Do not build it, but do not design it out either.

---

## 1. Vision & non-goals

**Vision.** A desktop app for one developer who has many open pull requests across one or more GitHub orgs that they choose. It answers two questions in one place: *"what state are my PRs in, grouped by the project they belong to?"* and *"who do I ping on Slack to get them reviewed?"* It also performs that ping (request, or bump an earlier request) in one click, with a preview before anything is sent.

PR Nexus reimplements, natively in Rust, the workflow of the existing Claude skill `request-slack-review` (§7). That skill is the behavioral reference. Deviations from it are listed explicitly.

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
│  └─ pr-nexus-providers/        # GitHub (blocking reqwest + GraphQL) and Slack (rmcp MCP client)
│                                # implementations of core traits; auth helpers (keyring, gh CLI).
├─ app/
│  ├─ package.json, vite.config.ts, src/ (React + TS + Tailwind)
│  └─ src-tauri/                 # bin crate `pr-nexus`: Tauri commands, app state, poll scheduler
├─ assets/slack-app-manifest.json # template for "Create your Slack app" (§7.11)
├─ docs/SPEC.md, AGENTS.md, CONTRIBUTING.md
├─ .changes/ (Changie), .changie.yaml, CHANGELOG.md
└─ .github/workflows/ ci.yml, nightly.yml, tag-release.yml, release.yml
```

**Why 2 library crates plus the app (and not the original 3–4):**
- `core` contains all logic that can be tested without the network: ownership rules, mapping precedence, message text, planner, SQL. It compiles fast and has no TLS stack. That keeps the edit-test loop for agents short.
- `providers` isolates heavy dependencies (reqwest, rustls, keyring, and later rmcp) and HTTP fixtures. It implements the traits from `core`.
- `app/src-tauri` stays a thin layer: it wires state and maps commands to services. It is a workspace member, so `cargo test --workspace` covers it.
- There is no `pr-nexus-mcp` crate in v1 (YAGNI). §10.1 describes where it will go.
- Splitting `adapters` into `vcs` and `chat` crates is rejected: 2 implementations do not justify 2 crates.

**Runtime model**
- A single Tauri process. Commands that touch the store or GitHub block on OS threads. The store does not use an async runtime, and neither does the GitHub client.
- Store: one SQLite file at the Tauri app-data dir `pr-nexus.db` (override with `PR_NEXUS_DB_PATH`), in WAL mode, `foreign_keys=ON`, `busy_timeout=5000`. Access goes through a `Store` that owns one write connection on a dedicated thread plus a small read pool. There is no `tokio` runtime and no `tokio-rusqlite` actor (§4). WAL already prepares for a second process, the *Future* MCP server.
- The frontend never runs SQL. Everything goes through typed Tauri commands. TS types are generated with `tauri-specta`. If the specta crates are unstable at bootstrap time, hand-write `app/src/bindings.ts` and add a test that keeps it in sync. Push events (`sync://updated`, `review://progress`) use Tauri events.
- `tauri-plugin-sql` is rejected, because it exposes raw SQL to the webview.

**Library choices (versions checked on crates.io, 2026-10-25)**

| Need | Choice | Version | Rationale |
|---|---|---|---|
| Desktop shell | `tauri` v2 | 2.12.x (CLI 2.12.4) | Stable v2. Capabilities-based permissions. |
| SQLite | `rusqlite` (`bundled`) | 0.41 | Sync API with simple ownership. sqlx 0.9 adds compile-time query checking (needs `DATABASE_URL`/offline prepare) and async-over-threadpool for SQLite, which buys nothing here. `rusqlite_migration` 2.6 pins rusqlite 0.40, and Cargo cannot link two `libsqlite3-sys` versions, so migrations are applied in-tree: `migrations/NNNN_*.sql`, one transaction, `PRAGMA user_version`. |
| GitHub | blocking `reqwest` (rustls) | 0.12 | GraphQL and the few REST calls are synchronous, on the same threading model as the store (§4). Queries are hand-written `.graphql` files with serde response structs. `octocrab` is async-first, so it is not used. `graphql_client` codegen is rejected to avoid vendoring the schema. Fixture tests guard the shapes. |
| Slack | `rmcp` client (`client`, `auth`, `transport-streamable-http-client-reqwest`) | 3.7 | Talks to the official Slack MCP server over Streamable HTTP, with client-side OAuth 2.1 + PKCE built in (§7.11). The same crate backs the *Future* MCP server (§10.1). Direct Web API (`slack-morphism` 2.31 or a thin `reqwest` client) is only the documented fallback. |
| Secrets | `keyring` | 3.6 | Service `dev.pr-nexus`, one entry `vault`. v3's `Entry` API is what the vault uses. Keychain calls run on one background thread and never block startup or the UI; the vault falls back to memory when the secret service is missing (§9), so Linux CI and a desktop without a keyring daemon still launch. |
| Errors | `thiserror` | 2.x | §4.3 |
| CSV | `csv` | 1.4 | mapping import/export |
| Frontend | React + TypeScript + Vite + Tailwind + TanStack Table + TanStack Query | — | Multi-select tables are the core UI. TanStack Table handles grouping and row selection. |
| JS tooling | Bun 1.3 (as in `harnesstap`) | — | Same toolchain as the user's other repo |
| *Future* MCP | `rmcp` (official Rust SDK) | 3.7 | §10.1 |

## 4. Domain model & SQLite schema

**Store threading.** `Store::open` applies migrations, then starts one writer thread named `pr-nexus-store`. That thread owns the only write connection. Reads borrow from a pool of four connections (`query_only`). A caller blocks until the writer replies. No async runtime, no task spawn, and no connection is shared across threads. `busy_timeout` covers the short lock a reader hits while the writer is in a transaction.

### 4.1 Rust types (pr-nexus-core::domain)

```rust
// ---- identity -----------------------------------------------------------
pub enum VcsKind { GitHub, GitLab, Gitea }        // only GitHub is implemented in v1 (§5.1)
pub enum ChatKind { Slack, Discord, Mattermost }  // only Slack is implemented in v1 (§5.1)

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
    pub hidden: bool,                         // closed PRs past sync.keep_closed_days
}

// ---- ownership (computed, cached per head_sha) ----------------------------
pub enum Tri { True, False, Unknown }
pub enum Readiness {
    Ready { notes: Vec<String> },             // does not block; e.g. waiting for reviews
    Blocked { reasons: Vec<String> },
    Unknown { reasons: Vec<String> },
}
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

**PR ↔ project cardinality: many-to-many.** A PR can belong to several projects (e.g. a shared-lib PR for two initiatives). In the grouped list it shows in each section, with a `+1` badge. Selection is keyed by `PrId`: checking it in one section checks it in every section, and bulk actions dedupe automatically (§8.2).

### 4.2 SQLite schema (migration 0001)

```sql
CREATE TABLE accounts (
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL CHECK (kind IN ('github','slack')),
  display_name TEXT NOT NULL, host TEXT NOT NULL,          -- github.com | <workspace>.slack.com
  external_id TEXT NOT NULL,                               -- GitHub user node id | Slack "T…:U…"
  slack_workspace TEXT UNIQUE,                             -- Slack accounts only: workspace domain (one account per workspace)
  login TEXT NOT NULL,
  auth_mode TEXT NOT NULL CHECK (auth_mode IN ('gh_cli','pat','oauth_device','slack_mcp_oauth','slack_user_token')),  -- slack_user_token: Web API fallback only (§7.11)
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

-- Message log: every Slack message PR Nexus posted or discovered. One message can cover several PRs (batch).
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
- Migration 0002 adds `pull_requests.search_text` (folded search text, §8.2, filled for older rows at startup) and the index `pr_projects_by_project` on `pr_projects(project_id)`. Migration 0004 clears it so startup rebuilds it with the current fold.
- Migrations live in `crates/pr-nexus-core/migrations/NNNN_*.sql` and run at startup in one transaction. The runner tracks `PRAGMA user_version` (same contract as `rusqlite_migration`; that crate is not a dependency, see §3).
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
    #[error("provider not supported: {0}")] Unsupported(String), // provider kinds without a v1 descriptor (§5.1)
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
pub trait VcsProvider: Send + Sync {
    fn kind(&self) -> VcsKind;
    fn viewer(&self) -> Result<VcsUser, ProviderError>;
    /// Open PRs the viewer authored or is requested on. Replaces org → repo → PR enumeration.
    fn search_pull_requests(&self, q: &PrQuery) -> Result<Vec<SyncedPullRequest>, ProviderError>;
    /// Refresh known PRs by provider id (catches merged/closed PRs that left the search results).
    fn refresh_pull_requests(&self, ids: &[ProviderRef]) -> Result<Vec<SyncedPullRequest>, ProviderError>;
    /// The slow status fields (review decision, mergeability, checks, threads) by provider id.
    /// Search leaves them out (§6); sync asks only for the PRs it keeps.
    fn pull_request_status(&self, ids: &[ProviderRef]) -> Result<Vec<PrStatus>, ProviderError>;
    /// Everything ownership resolution needs, for one PR (§6, §7.2). M9.
    fn ownership_inputs(&self, pr: &PullRequest) -> Result<OwnershipInputs, ProviderError>;
    fn team_members(&self, team: &TeamRef) -> Result<TeamMembers, ProviderError>;
    fn rate_limit(&self) -> Result<RateBudget, ProviderError>;
    /// Optional write (M10): request team reviewers on the PR.
    fn request_team_review(&self, pr: &PullRequest, teams: &[TeamRef]) -> Result<(), ProviderError>;
}

pub trait ChatProvider: Send + Sync {
    fn kind(&self) -> ChatKind;
    fn whoami(&self) -> Result<ChatUser, ProviderError>;
    fn list_channels(&self, cursor: Option<String>) -> Result<Page<ChatChannel>, ProviderError>; // picker
    fn channel_info(&self, channel_id: &str) -> Result<ChatChannel, ProviderError>;              // archived? name?
    fn post(&self, msg: &OutgoingMessage) -> Result<ChatMessage, ProviderError>;
    fn thread(&self, channel_id: &str, root: &ProviderRef) -> Result<Vec<ChatMessage>, ProviderError>;
    fn search(&self, query: &str) -> Result<Vec<ChatMessage>, ProviderError>; // Forbidden if unsupported
    fn permalink(&self, msg: &ProviderRef) -> Result<String, ProviderError>;
}

pub struct OutgoingMessage {
    pub channel_id: String, pub text: String,
    pub thread_root: Option<ProviderRef>,
    pub also_to_channel: bool,   // Slack: reply_broadcast. Ignored when thread_root is None.
    pub unfurl_links: bool,
}
```

All methods return typed errors (no `String`) and are synchronous, matching the store (§4). `SyncedPullRequest` is the search/refresh row: a `PullRequest` plus the repository node id, default branch, and archived flag the store needs. The original `list_repositories` is dropped: navigation is project-first (§8), and repos are discovered from PR search results. `list_organizations` is kept only to fill the org picker in onboarding/Settings, not for sync. It is the union of GraphQL `viewer.organizations`, REST `GET /user/orgs` (paginated, 100 per page), and orgs seen in search results. GraphQL silently omits orgs that enforce SAML SSO when the token is not authorized for them; REST still lists those memberships, so orgs found only through REST are stored in `github.sso_orgs` and shown with an “SSO authorization needed” hint and links to the org's SSO sign-in (`https://github.com/orgs/<org>/sso`) and GitHub token settings (their PRs do not sync until the token is authorized). A REST failure keeps the GraphQL list. Test doubles (`FakeVcs`, `FakeChat`) live in `pr-nexus-core::testing` behind a `testing` feature.

### 5.1 Provider registry and setup steps

The registry is the single place that knows which providers exist and what setting each one up takes. The setup wizard (§8.3), Settings → Accounts, and the sync scheduler all read it; nothing else matches on provider names.

```rust
pub enum SetupStep {                       // one wizard screen each (§8.3)
    ChooseVcs, ChooseChat,                 // always first
    ConnectVcs, ChooseVcsScope,            // from the VCS descriptor
    ConnectChat, ChatMapping,              // from the chat descriptor
    Done,                                  // always last
}

pub struct VcsDescriptor  { pub kind: VcsKind,  pub label: &'static str,
                            pub setup: &'static [SetupStep] }   // GitHub: [ConnectVcs, ChooseVcsScope]
pub struct ChatDescriptor { pub kind: ChatKind, pub label: &'static str,
                            pub setup: &'static [SetupStep],    // Slack: [ConnectChat, ChatMapping]
                            pub multi_account: bool }           // Slack: true (N workspaces)

pub struct ProviderRegistry { pub vcs: &'static [VcsDescriptor], pub chat: &'static [ChatDescriptor] }
impl ProviderRegistry {
    /// [ChooseVcs, ChooseChat] + vcs.setup + chat.setup + [Done]. Pure; unit-tested.
    pub fn setup_steps(&self, vcs: VcsKind, chat: ChatKind) -> Vec<SetupStep>;
}
// pr-nexus-providers (network code stays out of core): one factory per registered kind.
pub fn build_vcs(kind: VcsKind, account: AccountId, token: SecretString) -> Result<Box<dyn VcsProvider>, ProviderError>;
pub fn build_chat(kind: ChatKind, account: AccountId, token: SecretString) -> Result<Box<dyn ChatProvider>, ProviderError>;
```

- The descriptors and `setup_steps` live in `pr-nexus-core` (pure data, no network). The factories live in `pr-nexus-providers`, and the app wires them together.

- v1 registers only GitHub and Slack. The other `VcsKind` / `ChatKind` values stay in the domain so the store and traits remain extensible, but they have no descriptor (nothing reaches the UI, nothing is promised there) and their factories return `ProviderError::Unsupported`. Future providers are internal roadmap only (§10.2).
- `ChatMapping` and `ConnectChat` are the skippable steps (`SetupStep::is_skippable`). Connect Slack stays skippable after M10 so PR Nexus is usable before a workspace admin approves the Slack app (§7.13 R7); Skip records `slack_skipped` and never pretends a workspace is connected.
- Adding a provider later means adding a descriptor, its factory, and any new `SetupStep` screens; the wizard and Settings pick it up without other changes.
- The chosen providers are stored in `settings` (`providers.vcs`, `providers.chat`, §7.12). v1 allows exactly one of each.

## 6. GitHub sync

**Discovery uses GraphQL search, not org→repo enumeration.** The original design walked orgs, then repos, then PRs. That costs O(repos) calls and misses PRs in repos the user forgot to configure. Instead, each GitHub account runs two search buckets:

| Bucket | Query | Notes |
|---|---|---|
| `mine` | `is:pr is:open author:@me archived:false sort:updated-desc` + scope | primary |
| `review_requested` | `is:pr is:open review-requested:@me archived:false sort:updated-desc` + scope | "To review" filter, shipped in v1 (default D4) |

- Use `search(type: ISSUE, query:…, first: $first)` with cursor pagination and follow every page; there is no cap. GitHub returns at most 1000 results per query string, so when the first page's `issueCount` is above 1000 the query is split into two `created:` ranges (to the second), each checked again, until every range fits. Up to 2 ranges are paged at once. Each page goes to sync as it arrives. `ISSUE_ADVANCED` was removed on 2025-09-04, so do not use it.
- **Searches are scoped on the server.** The scope is ` org:NAME` for each tracked org (`github.tracked_orgs`, picked in onboarding and Settings → Accounts; §8), plus ` user:LOGIN` (the viewer) when personal repos are on. Several `org:`/`user:` qualifiers are ORed, so `org:acme org:acme-labs` matches PRs in either org. GitHub caps the query string at 256 characters, so `sync::search_queries` packs the qualifiers into as few strings as fit and the bucket runs one search per string; a PR found by two strings is kept once. Org names that are not plain logins (letters, digits, `-`, `_`, `.`) are dropped, and repeats are dropped case-insensitively. With no tracked org and personal repos off, nothing is searched. Nothing is hardcoded and there is no default org. Until the user picks orgs, the PR list shows an empty state with a "Choose orgs to track" button. In the UI the org list is also a filter chip, not a navigation level.
- **The client-side org filter stays as a safety net.** Results are still kept only for tracked orgs (and the viewer's own repos when personal repos are on), so a qualifier GitHub ignores cannot leak other orgs into the list.
- **Search is slim; status is a second query.** Mergeability (`mergeable`, `mergeStateStatus`) is by far the slowest part of a search page: on a busy public repo a page of 20 took about 7 s with it and 3 s without, and GitHub stops at about 10 s. Checks and review decision add more. So the search fragment (`ListFields` below) carries only the cheap fields, and sync then asks `nodes(ids:[…])` with `status.graphql` for review decision, mergeability, merge state, last commit (date and check rollup) and review threads, **only for the PRs it keeps** after the org filter. A PR the status query does not answer keeps the status already stored for it. Status chunks of 10 go out **2 at a time** (the per-host limit below), so status for a busy repo takes about half as long as one chunk after another. The readiness rules (§7) and the thread counts in the list need these fields, so they are not lazy.
- **Page sizes and GitHub timeouts.** Search pages, status chunks and refresh chunks start at **10** (`PAGE_SIZES` = 10, 5, 3); mergeability costs about 0.3 s per PR, so 10 stays well under GitHub's limit. When a page fails with 502/503/504, a GraphQL error that reports a timeout ("Something went wrong while executing your query…"), or a client timeout, the client waits (1 s, then 2 s), drops to the next smaller size for that page and the rest of the query, and tries again: at most 3 tries per page (10 → 5 → 3). Each GitHub request has a 15 s client timeout, so one page gives up after at most 3 × 15 s + 1 s + 2 s = 48 s. Other errors are not retried. When every try times out, the sync fails with a message that names the query and says GitHub timed out, for example `502: GitHub timed out on the search for review requests (3 tries, down to 3 per page). Try again in a minute.`
- **PRs that disappear** from a bucket (merged, closed, or no longer requested) are refreshed once via `nodes(ids:[…])` with the full fragment so their final state is stored. Only rows in a tracked org (or a personal repo with the toggle on) are refreshed. Merged/closed PRs leave the open list at that sync. The list's **Archived and closed** filter still shows them, with a Merged/Closed badge, for `settings.sync.keep_closed_days` (default 7); then they become `hidden`. The same sync closes their active review requests (`pr_merged` / `pr_closed`, §7.4).
- `scripts/prn-search-timing.sh` runs the same search and status queries with `gh api graphql` and prints only timings, for checking a heavy account (`--org NAME` once per tracked org, `--user LOGIN` for personal repos, `--sizes "10 5 3"`).

**Fragments** (one GraphQL file per query under `crates/pr-nexus-providers/src/github/graphql/`):

```graphql
# search.graphql
fragment PrListFields on PullRequest {
  id number title url bodyText isDraft state createdAt updatedAt
  author { login } headRefName baseRefName headRefOid
  repository { id owner { login } name isArchived defaultBranchRef { name } }
  labels(first: 20) { nodes { name } }
  reviewRequests(first: 20) { nodes { requestedReviewer {
    ... on User { login name avatarUrl(size: 64) }
    ... on Team { slug name avatarUrl(size: 64) organization { login } } } } }
}

# status.graphql: nodes(ids: $ids) { ... on PullRequest { … } }
  id state updatedAt reviewDecision mergeable mergeStateStatus
  commits(last: 1) { nodes { commit { committedDate statusCheckRollup { state } } } }
  reviewThreads(first: 100) { totalCount nodes { isResolved } }

# refresh.graphql: PrFields = PrListFields + the status fields
```

Unresolved threads come from counting `reviewThreads.nodes[].isResolved == false`. With more than 100 threads, show "100+".

**Polling** (no webhooks: a desktop app has no public endpoint):
- every 3 min while the window is focused, every 15 min while unfocused or minimised, and never while the app is quit;
- immediately when a PR detail view opens (that one PR, plus its ownership inputs if `head_sha` changed);
- **always** right before any Slack write for the affected PRs (§7.8);
- the manual refresh button in the sidebar footer (§8.2), which also shows sync status: `Synced · 2m ago`, `Syncing…`, or an error such as `Rate limited · resets 14:32` (time from `resetAt`, shown in local time) or `Sync failed · <reason>`.
- One sync at a time. A request that arrives while one runs (the startup sync, the scheduler, the refresh button, setup's Done step) waits for it and returns its status instead of starting a second one.
- Sync and every other app command run off the main thread (Tauri `async` commands that hand blocking work to the blocking pool), so a slow GitHub or Slack call never freezes the window.

**Incremental sync.** A sync is either *full* (a reconcile) or *incremental*.
- **High-water mark.** `settings["sync.mark"]` keeps, per scope (viewer login, personal repos, tracked orgs; lowercased, orgs sorted and deduplicated), the newest `updatedAt` seen, the last full pass, and the last all-open status refresh. It is written only after a pass succeeds, so a failed pass retries the same window. It moves to the newest `updatedAt` stored, never backwards.
- **Full** runs at app start, when there is no mark, when the last full pass is older than 6 h, when the scope changes (viewer or orgs), and on **Full resync** (sync details in the sidebar footer, with a tooltip). It runs the open-PR searches and refreshes local open rows that no search returned, as before.
- **Incremental** runs the same searches with `is:open` dropped and `updated:>=<mark − 5 min>` added (UTC, the overlap covers clock skew and search index lag), split by date buckets exactly like the full search. A hit that is now merged or closed moves its stored row; a closed hit with no local row is ignored.
- **Volatile fields.** `reviewDecision`, `mergeable` and the check rollup can change without `updatedAt` moving. Every incremental pass re-reads them with `status.graphql` (`nodes(ids:)`, chunks of 10, since mergeability costs about 0.3 s per PR) for open rows whose checks are pending or expected or whose mergeability is unknown. Every 30 min it re-reads all open rows. A status answer whose `state` is final or whose `updatedAt` is newer than the stored row triggers a full `refresh.graphql` for that row.
- **Changed rows only.** A row or repository that matches what is stored is not written. The sync status carries a `revision` that bumps only when stored rows changed; the list re-reads on a revision change (polled every 500 ms while syncing and every 15 s otherwise). The footer shows `Synced · 3 updated · 2m ago` after an incremental pass that changed rows.
- **Cost.** For about 100 open PRs with nothing changed, a full pass costs about 20 requests and 200 row writes. An incremental pass costs 2–3 requests and no writes (plus 10 status requests on the 30-min refresh).

**Live updates.** Between syncs, a background thread watches the notifications inbox so a pull request is refreshed within about a minute of a change. It is on by default (`live.enabled`, Settings → Sync → Live updates) and only ever reads: threads are never marked as read and no PATCH or PUT is sent.

- **Inbox poll.** `GET /notifications?all=true&participating=false` with `If-Modified-Since` set to the last `Last-Modified`. An unchanged inbox answers 304, which does not count against the rate limit. `all=true` includes threads the user already read on the web, since those pull requests changed too. The next poll waits for `X-Poll-Interval`, never less than 60 s. Up to 3 pages are followed while every thread on a page is new.
- **Targeted refresh.** Threads newer than the stored watermark whose subject is a `PullRequest` (`…/repos/{owner}/{repo}/pulls/{n}`) are coalesced by pull request and each one is read with one GraphQL `repository.pullRequest` query, one at a time. Other subjects are ignored. A row already stored keeps its involvement plus anything the thread's reason adds (`review_requested`, `author`). A pull request not stored yet is added only when it is open, in a tracked org (or personal and opted in), and the viewer is involved. Only changed rows are written, and a change bumps the list revision.
- **Watermark.** `live.mark` stores `Last-Modified` and the newest thread `updated_at` handled. It is saved only after every targeted refresh succeeds, so a failure re-reads rather than skips. The first run starts at the current time: the start-up full sync covers the backlog.
- **Pending checks.** Open pull requests whose checks are pending or expected have their status re-read with the batched status query (10 per request) every 90 s while the window is focused and every 5 minutes otherwise. A status that finished or moved beyond its checks triggers a targeted refresh.
- **Focus.** Coming back to the window after more than 60 s away runs an incremental sync, at most once per 30 s.
- **Tokens.** gh CLI and OAuth tokens and classic PATs with `notifications` or `repo` work. Fine-grained PATs and GitHub App tokens are recognised by prefix and never polled; a 403 or 404 from the inbox, or a classic token without the scope (`X-OAuth-Scopes`), disables the inbox poll once with "Live updates unavailable with this token type: falls back to periodic sync" (or a scope note) in the sync details panel, without toasts or retries. Pending checks still refresh. Signing in with another token re-enables it.
- **Failures.** Errors back off from 60 s to 15 minutes; a rate limit waits for its reset. Live work pauses with scheduled polls under 10% of the budget and skips a pass while a sync is running.
- The sync details panel shows "Live updates on · checked 2m ago", "Live updates off", or the reason it is unavailable.

**Rate limits.**
- Read `rateLimit { remaining resetAt cost }` in every GraphQL query and the REST `x-ratelimit-*` headers.
- Store them in `sync_state`.
- When remaining is below 10% of the limit, skip scheduled polls until `resetAt`; manual and pre-send refreshes still run.
- On a secondary limit (403/429 with `retry-after`), back off exponentially starting at `retry-after` (each wait at most 2 min) and retry the page, up to 3 times; then return `RateLimited{secondary:true}`.
- Send at most 2 concurrent requests per host.

**Ownership inputs.** These are fetched lazily: for PRs the user opens or selects for a request, plus a capped post-sync pass for BLOCKED PRs with no review decision (below). They are cached in `ownership_cache` keyed by `(head_sha, base_ref, codeowners_oid)`.

| Input | Source | Failure → |
|---|---|---|
| Changed files | GraphQL `files(first:100)` paginated; cap 3000 (GitHub's own limit) | `truncated=true` → confirmation (§7.1) |
| CODEOWNERS | `repository.object(expression: "<base>:.github/CODEOWNERS")`, then `CODEOWNERS`, then `docs/CODEOWNERS`, aliased into one query. The first hit wins (GitHub order). | not found → no CODEOWNERS (user must pick teams) |
| Code-owner requirement | REST `GET /repos/{o}/{r}/rules/branches/{base}`: `pull_request.parameters.require_code_owner_review`. This returns the rules that **actually apply** to the branch, including org rulesets, and replaces the skill's hand-written ruleset matching. | error → `Unknown` |
| Classic branch protection | REST `GET /repos/{o}/{r}/branches/{base}/protection/required_pull_request_reviews` | **403/404 for non-admins is normal** → treat as `Unknown`, not `False` |
| Effective approvals | `latestOpinionatedReviews` (fragment) | — |
| Team membership | REST `GET /orgs/{org}/teams/{slug}/members` (paginated), cached 24 h | 403 → attribution unknown for that team |

`codeowners_required` = `True` if either source says true. It is `False` if `rules/branches` says false **and** protection is known to be false or absent (404 when the caller is an admin). Otherwise it is `Unknown`.

**Probe, inputs, and cache (M9).**
- Each resolve starts with one GraphQL **probe**: `headRefOid`, `baseRefName`, the three aliased CODEOWNERS blobs (`oid`, `text`, `isTruncated`), `latestOpinionatedReviews(writersOnly:true)` with `onBehalfOf`, and `viewerPermission`. A truncated CODEOWNERS blob is a load error, not a partial file.
- The probe gives the cache key `(head_sha, base_ref, codeowners_oid)`. Files, `rules/branches`, and protection are fetched only on a cache miss, so a cached PR costs one call. Reviews are never cached; they change without a new head.
- Team membership is fetched only for candidate teams whose approvals lack `onBehalfOf`, at most 20 teams per resolve, and cached 24 h in `team_members_cache`. 403/404, more than 10 pages, or a transient error give attribution unknown for that team. Only complete lists are cached. Rate-limit and auth errors stop the resolve.
- Classic protection that the caller cannot read stays `Unknown` and never turns the requirement into `False`. A 404 means "absent" only when `viewerPermission = ADMIN`.
- **Null review decision.** When `mergeStateStatus = BLOCKED`, `reviewDecision` is null, and nothing else blocks, readiness depends on the code-owner status from the cache. A code-owner review known to be pending gives Ready with the BLOCKED note. An unknown status gives Unknown (§7.1). A known non-pending status gives the generic BLOCKED Unknown.
- The list reads cached facts only. Opening the PR detail resolves that PR off the UI thread. After each sync, PRs whose list readiness is the null-decision Unknown are resolved too, at most 10 per sync. A rate-limit or auth error stops the pass. This keeps the null-decision rule from leaving the list Unknown until each PR is opened.

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
| mergeStateStatus BLOCKED, `reviewDecision` null, otherwise clean, code-owner review status not known yet | Unknown | `GitHub reports BLOCKED with no review decision; code-owner review status is unknown` |
| mergeStateStatus BLOCKED otherwise | Unknown | `GitHub reports BLOCKED for a reason other than missing reviews` |
| file list truncated (≥3000) | Unknown | `PR file list may be truncated (≥3000 files); confirm additional owners` |
| evidence fetch failed | Unknown | the provider error |

- **Blocked** PRs: the Slack action button (§7.7) is disabled and the tooltip lists the reasons. Blocked PRs cannot be overridden in v1. The skill says "never override", and a red CI is a bad reason to page a team.
- **Unknown** PRs: the preview shows the reasons and requires an explicit "I confirm only reviews are missing" checkbox. The confirmation is scoped to `(head_sha, reasons)`. If either changes, the confirmation is invalidated.
- **Δ BLOCKED: intentional deviation from the skill (decided by the owner).** The skill marks every `BLOCKED` PR as Unknown. Nearly every PR that waits for review reports `BLOCKED`, so the skill asks for confirmation on almost every request. PR Nexus treats a `BLOCKED` PR as **Ready** when only reviews are missing:
  - `reviewDecision = REVIEW_REQUIRED` (or null with code-owner review pending);
  - `mergeable = MERGEABLE`;
  - the check rollup is SUCCESS, with every check passing, neutral, or skipped;
  - no blocking row above applies (not a draft, not behind, no changes requested).

  The PR detail and preview show a note: "GitHub BLOCKED: waiting for reviews". Any other `BLOCKED` stays Unknown. Other GitHub requirements (e.g. signed commits, deployments) can still hide behind `BLOCKED`. The owner accepted that risk: the worst case is a review request that arrives slightly early. Golden tests must cover both branches.
- **`review.blocked_review_required_is_ready`** (default `true`, §7.12) gates the deviation above. `decide_readiness_with(snapshot, flag)` keeps the Ready note when the flag is true. When the flag is false, that same PR is Unknown and the note is a reason, which is the skill's behavior. Callers read the setting from the store; the function stays pure.
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
3. A row with `slack_channel_id`: use it. Check it once per session with `slack_search_channels`, which returns the archive status. If the channel is **archived**, refuse to post and open the picker with a banner: "#x is archived — pick its replacement. It will be saved as your correction." (The skill followed redirect messages by hand. The app does not parse "moved to" messages; the saved correction replaces the old channel.)
4. A row with only `slack_channel_name`: resolve it through the cached channel list (`slack_list_user_channels`, public+private, archived hidden; `slack_search_channels` for channels the user is not in). The user confirms the match once, and it is saved as a user row with an ID.
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
- **`source='discovered_via_search'`.** Requests posted outside the app (by hand, or by the skill) are found with the Slack MCP search tools (`slack_search_public`, then `slack_search_public_and_private` when the channel may be private; always `include_bots=true`, sorted by timestamp) and stored the same way.
  - When it runs: in the background, once per open authored PR that has no active request, per connected workspace, at most every 24 h (`discovery_runs`), throttled to 10 calls/min per workspace (search has special rate limits, §7.11). It also runs right before opening the preview for any selected PR with no active row (bypassing the 24 h cooldown), and when the user clicks "Search Slack".
  - Background query (the target channels are not known yet): `"github.com/ORG/REPO/pull/N"` across the workspace. Pre-preview query, once channels are resolved, uses the skill's exact template: `in:<#CHANNEL_ID> "github.com/ORG/REPO/pull/N"`.
  - Match hits whose text contains `https://github.com/ORG/REPO/pull/N` followed by a non-digit or end of text (`/pull/4` must not match `/pull/42`). Keep only hits in channels that some mapping row points to, or that the user picks in the preview.
  - Per (PR, workspace, channel), the oldest matching top-level message is the root: the hit's `thread_ts` if it is a reply, else its `ts`. `requested_at` = root time. Later matching messages in the same thread count as bumps (`last_bumped_at`, `bump_count`).
  - A hit whose root matches a **closed** row is ignored. The `UNIQUE` key prevents resurrection, so a dismissed request stays dismissed.
  - If the search scopes are missing (`search:read.public`), discovery is skipped and the PR shows "Slack history: app only". Without `search:read.private`, only public channels are searched.

**When a request stops being active (closed).** This decides whether the action is Request or Bump.

| Event | Effect |
|---|---|
| The PR is merged or closed (seen by sync) | all its active rows → `pr_merged` / `pr_closed` |
| The user clicks **Dismiss request** (PR detail, per channel) | that row → `dismissed`. The next action for that channel is a new Request. |
| A new top-level message is posted to the same (workspace, channel) for this PR (user chose "new top-level message", or the >3-roots fallback) | old row → `superseded` |
| The channel is archived (seen by the archive-status check, §7.3) | row → `channel_archived`. The next action is a Request to the replacement channel. |
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
- **Δ:** the skill's `upsert-mapping.py` writes CSV columns in a nondeterministic order (it builds them from a Python `set`). PR Nexus always exports this fixed header:
  `github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id`
  plus an optional 6th column, `slack_workspace` (a workspace domain). The skill's 5-column CSV imports as is. On import, rows without a workspace get the **target workspace the user picks in the import dialog** (required when any row lacks one). The choice is a select of the connected workspaces, shown as name and host (`Acme (acme.slack.com)`), with an expired sign-in still listed and marked "needs sign-in"; its value is the domain. One workspace is preselected; with several, the last one used (kept per machine), else the first. With none connected the select is disabled and links to Settings → Accounts (wizard: back to step 5), and Preview stays disabled unless the CSV has a `slack_workspace` column. The row editor in Settings → Mappings uses the same select and keeps a row's workspace that is no longer connected. A host or URL in the CSV (`acme.slack.com`) is stored as its domain. A domain with no connected Slack account still imports. That row shows "workspace not connected" and is skipped when sending.
  The export quotes with RFC 4180 rules and sorts rows by `(org, slug)`, so exports are diff-friendly.
- **Tombstones do not survive CSV.** A user row with `disabled=1` is omitted from every export mode (`effective`, `user`, `imported`). The CSV has no disabled column, and import always writes `disabled=0`. Exporting and importing therefore cannot recreate a tombstone. The row stays in SQLite until the user deletes that user-layer row in the app.
- Import modes:
  - `replace`: delete imported rows, then insert.
  - `merge`: upsert into user rows; empty cells do not overwrite non-empty ones.
  - A dry-run diff is shown before applying: added, changed, unchanged, and rows that the user layer shadows.
  - Unknown columns are an error. Missing optional columns default to empty.
- **CSV source.** The text is pasted, or loaded with **Choose file…** (wizard step 6 and Settings → Mappings). That button opens the native open dialog through `tauri-plugin-dialog` (filters: CSV, All files) and only returns a path to the webview; the `read_mapping_file` command reads it in Rust (regular file, at most 1 MiB, UTF-8, a leading BOM dropped) and fills the CSV box with the text and the file name. The usual import commands then parse and validate it, so a file and a paste fail the same way. The app has no `fs` plugin scope.
- Export modes: `effective` (merged view), `user` (corrections only, for backup), and `imported`.
- **No company data in the repo.** Test fixtures use made-up orgs, teams, and channels (`acme`, `team-a`, `#team-reviews`). The CSV parser is tested on those fixtures (header, column count, unique `(org, slug)`, `C…`/`G…` ID shape).
- The settings UI has a mappings table with filters (org, empty channel, overridden) and inline edits that write user rows. It has a "Reset to imported" action per row, which deletes the user row.

### 7.6 Message text: byte-exact port of `build-review-message.py` (MUST)

`pr-nexus-core::review::message::build(mode, items: &[(title, url)], requester: Option<&str>, extra: Option<&str>) -> Result<String, CoreError>`. An empty item list, an empty title, or a URL that does not start with `http` is `Err`. Rules:
- Titles and URLs are `trim()`med. An empty title is an error. A URL must start with `http`.
- Every `{title}` below is rendered as a Slack inline code span, `` `title` ``, so Markdown characters in it (`_`, `*`, `~`, `[`…) show literally. Backticks inside the title become `'`; a title that is already wrapped in backticks is not wrapped twice. `&`, `<`, `>` stay raw on the MCP path (literal inside a code span) and are entity-escaped by the Web API fallback.
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

- The bare URL always sits on its own line. Never use `<url|title>` or Markdown links; the only formatting is the title's code span.
- Golden tests: fixtures were generated by running the Python script (`uv run scripts/build-review-message.py …`) for about 12 cases: single/batch × request/remind × requester/extra, unicode, titles with `&<>`, and whitespace. Commit the expected outputs under `crates/pr-nexus-core/tests/fixtures/messages/`. Rust output must equal them byte for byte. They now differ from the script only by the backticks around each title.
- **Wire format: standard Markdown for `slack_send_message`.** The MCP send tool takes standard Markdown, not Slack mrkdwn, and converts it on the server. The skill sends the script's stdout **unchanged** through that same tool, so the golden text already is what the tool expects: plain lines, `- {title}` as a list item, bare URLs on their own line (auto-linked), and `:pray:` as an emoji shortcode.
  - `review_messages.text`, the preview, and the sent text are the same string (`wire_text` is the identity).
  - **No mrkdwn entity escaping** (`&amp;`, `&lt;`, `&gt;`) on the MCP path; it would show literally. That escaping belongs to the Web API fallback only (§7.11).
  - Titles are code spans, so no title escaping or Markdown-character warning is needed. Earlier builds warned on any title with `_ * ~` etc.; that warning is gone. The optional note is the sender's own text and is sent as written.
  - The preview renders the same Markdown subset that Slack shows.
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
| Slack call | `slack_send_message(channel_id, message, unfurl_app_links=true)` | `slack_send_message(channel_id, message, thread_ts=<root_ts>, reply_broadcast=<review.bump_broadcast>)`: by default posted in the thread **and** in the channel, same as the skill |
| Stored | `review_messages(kind='request')` + new active `review_requests` rows (`source='app'`) | `review_messages(kind='bump', thread_root_ts=<root>)` + `last_bumped_at`, `bump_count` on the active rows |
| Guard | — | Warn (not block) if the last request or bump in this channel is less than `review.min_bump_interval_hours` old (default 24, default D12). |

Because bumps use `reply_broadcast=true` by default (`review.bump_broadcast`, §7.12), Slack also shows each bump in the channel or DM history: `slack_read_channel` lists it as a top-level message while the root reports `Thread: N replies`. This is intended (the skill does the same), not a duplicate post. Warnings and toasts name a DM target `DM` (or `DM · name`), never `#D…`. A successful single send closes the dialog and shows a toast (`Review requested in acme · DM`, with **Open ↗** to the permalink). A bulk run shows a summary toast (`Slack: 2 sent · 1 failed`) when its result dialog closes.

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

- Select PRs with the three-level tri-state checkboxes (global, per project section, per row; §8.2). A floating bulk bar appears at the bottom: `N selected · [Request / bump on Slack] · [Clear]`. There is **one** bulk action. The planner sorts each PR into a request or a bump per channel, using `review_requests`.
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
- **Pacing (M11).** Sends are sequential. Posts to the same channel are at least 1.1 s apart (Slack allows about one message per second per channel). On `RateLimited` the sender waits the `Retry-After` once when it is 60 s or less; otherwise the item fails with the time to retry.
- **Targets.** A typed channel or DM ID (`C…`, `G…`, `D…`) is accepted wherever a channel ID can be entered, including "All to one channel" in the bulk preview. A DM target reads as `DM` (or `DM · <typed name>`), never `#D…`. The DM user's name is not resolved: that needs `im:read` and `slack_list_user_channels`, which PR Nexus does not request.
- **Sent, link unknown.** When Slack accepted a message but neither its result nor the recovery read (§7.11, Parsing) yields a `ts`, the item is `sent` with detail "Sent, link unknown…": it has no permalink, is not tracked in `review_requests`, and **Retry failed never resends it**. Reporting it as failed would invite a duplicate post.

### 7.10 Chat context panel (scoped read)

The PR detail has a collapsed section titled "Slack threads and mentions" (its latest request is also summarised above it, §8). It shows **only** threads PR Nexus knows about for this PR: the roots of its `review_requests` rows (active first, then closed, labeled with the close reason) and their `review_messages`. For each thread root, it calls `slack_read_thread(channel, ts)` (the first 50 replies are shown) and caches the result in `chat_thread_cache` for 5 min. It renders replies with the author's name (`slack_read_user_profile`, cached), time, and text, and links to the permalink. The card header always shows what the DB knows, the same status as the list row (`Bumped · 9h ago` / `Requested · 2d ago`, with the request time and bump count on hover) and the permalink, so a signed-out workspace still shows the last action, the last cached copy of the replies (if any), and an "Open ↗" link.
- There is no channel browsing, no history crawling, and no posting from the panel in v1. (Posting is only through the Slack preview.)
- Optional "Mentions" (off by default): `slack_search_public` (or `slack_search_public_and_private`) for `"github.com/ORG/REPO/pull/N"` with `include_bots=true` across the workspace, limited to 20 results, to find discussions in other channels. It is one call per click, not polled.

### 7.11 Slack transport: the official Slack MCP server

**Transport (v1).** PR Nexus talks to Slack only through Slack's official remote MCP server, `https://mcp.slack.com/mcp` (setting `slack.mcp.url`). PR Nexus is an **MCP client** of that server and makes no direct Slack Web API calls in v1. Messages are posted *as the user* with a user token. This matches the skill, which also goes through the Slack MCP server.

Facts checked on 2026-10-25 (https://docs.slack.dev/ai/slack-mcp-server and the server's discovery documents):
- Transport: JSON-RPC 2.0 over Streamable HTTP only. No SSE, and **no Dynamic Client Registration**.
- App identity: every MCP client must be backed by a registered Slack app with a fixed client ID, which workspace admins approve through the normal app approval process. Only Marketplace-published apps and internal apps may use MCP.
- Discovery: `/.well-known/oauth-protected-resource` names the authorization server `https://mcp.slack.com`. Its metadata lists:
  - authorize endpoint `https://slack.com/oauth/v2_user/authorize` and token endpoint `https://slack.com/api/oauth.v2.user.access`;
  - `code_challenge_methods_supported: [S256]`;
  - grants `authorization_code` and `refresh_token`.
  An unauthenticated call returns `401` with `WWW-Authenticate: Bearer resource_metadata=…`.
- Client authentication: the docs describe confidential OAuth (`client_id` + `client_secret`) and say PKCE is available for desktop clients. The metadata lists only `client_secret_post` as the token-endpoint auth method. Slack's PKCE guide (https://docs.slack.dev/authentication/using-pkce) adds:
  - an app is a public client only after `oauth_config.pkce_enabled` is turned on, which is one-way;
  - only then are `localhost` redirects treated as desktop redirects, whose code exchange and refresh need no `client_secret`;
  - refresh tokens of PKCE-enabled apps expire after 30 days.
  So PR Nexus supports both modes. It always sends PKCE S256, and sends the **client secret when one is configured**. An app created from PR Nexus's manifest is confidential (secret required). A client ID alone works only for an app that has PKCE enabled. M10's spike proves both exchanges against mocks, and the PKCE-only exchange live (§7.11 spike result, §7.13 R2).
- Rate limits: the Web API limits of the underlying method apply, per tool:
  - send message: special (about 1 per second per channel);
  - search messages: special;
  - search channels, list user channels, search users: Tier 2;
  - read thread, read channel: Tier 3;
  - read user profile: Tier 4.
  The limits are tracked per app, so they are shared with any other client that uses the same client ID.

**Client library: `rmcp` (verified, client-side OAuth supported).** Use `rmcp` 3.7 (crates.io, 2026-10-22) with the features `client`, `auth`, and `transport-streamable-http-client-reqwest`. Its `transport::auth` module implements client-side OAuth 2.1:
- PKCE S256;
- protected-resource discovery (RFC 9728) and authorization-server discovery (RFC 8414);
- RFC 8707 resource binding, and scope selection from `WWW-Authenticate`;
- automatic token refresh;
- `AuthClient`, which wraps the streamable HTTP transport. It refreshes once on a rejected token and otherwise surfaces `AuthRequired`.

Registration follows the MCP spec's priority order: a **pre-registered client** (`AuthorizationRequest::with_preregistered_client(client_id)`, plus `with_client_secret(secret)` when a secret is configured) wins over Client ID Metadata Documents and DCR. That is exactly what Slack needs, because it has no DCR. Tokens go through a custom credential store backed by the keychain vault (§9). No hand-written OAuth is needed.

*Fallback if the spike shows a gap* (for example, rmcp refusing to exchange without a secret because the metadata lists only `client_secret_post`, or not exposing the token response's `team`/`authed_user`): run the authorization-code + PKCE exchange by hand with `oauth2` 5 against the two endpoints above (a small module). Then give the access token to the rmcp transport as a static bearer header, and refresh on `AuthRequired`. The rest of the design does not change.

**M10 spike result (2026-10-26).** What was verified, and how:
- *Verified against mocks* (wiremock MCP server, discovery documents, and token endpoint in `crates/pr-nexus-providers/tests/slack_mcp.rs`): rmcp 3.7 discovers the authorization server from the `401` + `resource_metadata`, builds the authorize URL with the pre-registered client ID, `state`, PKCE S256, the user scopes, and the configured `redirect_uri`, and exchanges the code.
  - **Without a secret** it sends a public-client exchange: `client_id` + `code_verifier`, no `client_secret`. rmcp does not refuse the exchange because the metadata lists only `client_secret_post`.
  - **With a secret** it sends `client_secret_post` plus the PKCE verifier.
  - Slack's token response shape (`token_type: "user"`, `team`, `authed_user`, optional refresh token and expiry) parses, and refresh reuses the stored grant.
  - The loopback listener binds the configured host and port, checks `state`, maps `error=access_denied` to a denial, times out after 10 minutes, honours Cancel, and reports a port already in use.
- **Verified live (2026-10-26, real Slack workspace, PKCE-enabled app, no client secret, custom loopback redirect URI):** Slack accepted the authorize request on `oauth/v2_user/authorize` (user scopes in `scope`, PKCE S256, the RFC 8707 `resource` parameter), redirected to the configured loopback URI with a code, and accepted the **PKCE-only token exchange** (`client_id` + `code_verifier`, no secret). `auth.test` with the MCP-issued user token returned the workspace name, domain, and user, so the connected row showed team · `domain.slack.com` · `@user`. The first attempt failed only because signing in took more than the original 2-minute wait (see step 3).
- *Not verified live yet:* PKCE parameters alongside a secret on a manifest-created (confidential) app, and refresh behaviour (needs a rotating token to expire).
- **Answer to "does PKCE without a secret work":** yes, end to end with real Slack on an app with PKCE enabled (verified live, above). rmcp performs a public-client PKCE exchange when no secret is set. Only an app with `pkce_enabled` turned on is a public client (see above); a manifest-created app without PKCE enabled still needs the secret.
- **Fallback, in order:** (1) enter the client secret in the wizard or Settings (stored in the keychain) and sign in again; (2) if Slack rejects the PKCE parameters or the `resource` parameter with a secret, switch to the manual exchange described above (`oauth2` against the two endpoints, the access token given to rmcp as a static bearer). The error shown on `invalid_client` / `bad_client_secret` names the missing secret.
- **Identity:** the token response's `team` and `authed_user` give the IDs. PR Nexus then calls the Web API `auth.test` once with the new user token to learn the workspace name, domain, and user name. This is the one documented exception to "no direct Web API calls": the MCP server has no tool that returns the workspace domain. If `auth.test` fails, the account is still saved, named after the team ID, and the domain is filled from the first permalink.

**OAuth flow, one workspace at a time** (wizard step 5, §8.3, or Settings → Slack):
1. **Client settings are user data.** Read `slack.mcp.client_id` and `slack.mcp.redirect_uri` (§7.12): global, with an optional per-workspace override.
   - The redirect URI is a loopback URI that must exactly match one registered on the Slack app, e.g. `http://127.0.0.1:<port>/mcp/oauth/callback`. The port is part of the match.
   - No client ID ships with the app, the repo, fixtures, or docs; the wizard asks for it (D20).
   - **Client secret: a first-class optional field** next to the client ID. It is needed for a confidential app, which includes every app created from PR Nexus's manifest, and left empty for a PKCE-enabled public app. It lives in the keychain vault (`slack-client:<client_id>`, §9), never in `settings`, logs, or the webview after it is saved: the field then shows "Saved in keychain · [Replace] [Clear]".
2. **Listen and open the browser.** PR Nexus binds a one-shot HTTP listener on exactly that host and port (`127.0.0.1` only). It builds the authorize URL with `state`, PKCE S256, `redirect_uri`, and the user scopes below in the **`scope`** parameter, and opens it in the system browser from Rust (on Linux `xdg-open`, so the desktop's default browser; otherwise the platform opener). Slack shows its workspace picker and consent screen, and the user picks one workspace.
   - **`scope`, not `user_scope`.** Discovery points at Slack's user-token endpoints, `https://slack.com/oauth/v2_user/authorize` and `oauth.v2.user.access` ([Slack MCP server overview, "OAuth URL and endpoints"](https://docs.slack.dev/ai/slack-mcp-server/#oauth-endpoints)). On that endpoint `scope` carries the user's own scopes: Slack's example is `https://slack.com/oauth/v2_user/authorize?client_id=YOUR_CLIENT_ID&scope=chat:write,channels:history`, and the flow "focuses exclusively on the user's identity and personal permissions" ([Installing with OAuth, "The user-centric flow"](https://docs.slack.dev/authentication/installing-with-oauth#v2-user-access)). `user_scope` only belongs to the `oauth/v2/authorize` install flow, where `scope` means bot scopes. PR Nexus therefore sends the user scopes in `scope` and never sends `user_scope`; a test asserts both. An empty `user_scope=` seen in Slack's own workspace sign-in redirect is Slack carrying the original query through its login pages, not something PR Nexus sends.
3. **Callback.** PR Nexus checks `state` and exchanges the code. The listener closes after the first valid callback, on Cancel, or after 10 minutes (`CALLBACK_TIMEOUT`).
   - Why 10 minutes: in a live test Slack accepted the request and redirected with a code, but the user needed more than 2 minutes to sign in to the workspace (password, SSO, 2FA), and the closed listener gave `ERR_CONNECTION_REFUSED`.
   - While waiting, the status line counts down ("Finish signing in in your browser · m:ss left."), switches to "Still waiting for Slack." after 30 s, and keeps **Cancel**. On timeout the panel shows "Sign-in timed out, click to retry.", which starts a fresh sign-in.
   - If the port is already taken, the step says so ("Port <port> is in use. Finish or close the other app's Slack sign-in, or change the redirect URI.") instead of falling back to another port, which Slack would reject.
4. **Identity.** The token response carries the user access token (`xoxp-…`). When the app has token rotation, it also carries a refresh token (`xoxe-…`) and an expiry. `team.id` and `authed_user.id` identify the account; `auth.test` adds the workspace name, domain, and user name (see the spike result above). The account row gets `auth_mode='slack_mcp_oauth'`, `external_id='T…:U…'`, the team name as display name, and the domain as `slack_workspace`. Signing in to the same workspace again updates that row.
5. **Storage.** The access token, refresh token, and expiry go into the keychain vault under `slack:<team_id>:<user_id>` (§9).
   - Refresh is automatic.
   - A failed refresh, `invalid_auth`, or `token_revoked` maps to `Unauthorized`. The UI shows "re-auth <workspace>", which reruns this flow for that workspace only.
   - **Startup check.** An account row alone proves nothing: with the keychain disabled the vault is in memory and loses grants on restart. At startup (and on the first workspace list) each connected account is checked: no grant in the vault → *needs sign-in*; `auth.test` answering `invalid_auth`, `not_authed`, `token_revoked`, or `account_inactive` (or `token_expired` without a refresh token) → *needs sign-in*; a network error leaves the state unknown and the account usable. A send that returns `Unauthorized` also marks it. Settings shows "needs sign-in" with a per-workspace **Reconnect** button that reruns the flow and updates the same row; **Add another workspace** is only for new workspaces. If Slack signs in to a different workspace than the one being reconnected, both rows are kept and the notice says so.
   - **Remove workspace** deletes the local tokens and account row. It never calls `auth.revoke`, because the grant may belong to an app shared with another client (§7.13 R1). When review requests still reference the account, the row stays as "signed out" (tokens deleted) so the history keeps its workspace; signing in again restores it.

**User scopes requested** (the minimum; the Slack app must have them as user scopes):
- `chat:write`;
- `search:read.public`, plus `search:read.private` (optional, for private-channel discovery);
- `channels:read`, `groups:read`;
- `channels:history`, `groups:history`;
- `users:read`;
- `search:read.users`, for the target picker's people search (`slack_search_users`). Apps created before this scope was added must add it under **OAuth & Permissions → User Token Scopes** (or be recreated from the manifest) and reconnect; until then people search fails with `missing_scope`, the picker says so, and channel search and ID entry keep working.

The MCP search tools use the `search:read.*` scopes, not the legacy `search:read`. If the app lacks an optional scope, the matching feature degrades as described in §7.4 and §7.10.

**Tool mapping.** `SlackMcpChatProvider` in `pr-nexus-providers` implements `ChatProvider` (§5) with MCP `tools/call`. Tool names and input fields are those listed by the server on 2026-10-25.

| `ChatProvider` | Slack MCP tool | Arguments and notes | Limit |
|---|---|---|---|
| `post` | `slack_send_message` | `channel_id`, `message` (**standard Markdown**, not mrkdwn; §7.6), `thread_ts`, `reply_broadcast`, `unfurl_app_links`. The tool description says only "return the message link to the user"; the result format is below. | special |
| `permalink` | none: taken from or built from results | A link in the result wins: `ts` is parsed from `/archives/<C…\|G…\|D…>/p<16 digits>` (`p1700000000000100` → `1700000000.000100`), and `thread_ts` from the query string. Otherwise the link is built from the workspace domain, channel, and `ts`: `https://<domain>.slack.com/archives/<channel>/p<ts without the dot>`, plus `?thread_ts=<root>&cid=<channel>` for a reply. There is no permalink tool. Discovered messages carry their own link from search. | — |
| `search` | `slack_search_public`, then `slack_search_public_and_private` when the target channel may be private | keywords plus `in:<#C…>`, `from:`, `after:` filters, `include_bots=true`, sorted by timestamp | special |
| `list_channels` | `slack_list_user_channels` (channels the user is in; `team_id` on Enterprise Grid), plus `slack_search_channels` for the picker's search box | Archived channels are included by default. The picker hides them. | T2 |
| `search_channels` | `slack_search_channels` (`keywords` from the term, `natural_language_query` empty, `channel_types=public_channel,private_channel`, `include_archived=false`, `limit` ≤ 20, `response_format=concise`) | The target picker's `#` search. The concise result lists `#name (C…) - <type> - …`; archived rows and DM IDs are dropped. | T2 |
| `search_users` | `slack_search_users` (same fields, no channel types) | The target picker's `@` search. Each line is `Name - U… - email - title`; only the name and user ID are kept (emails are never stored). Needs `search:read.users`. | T2 |
| `channel_info` | `slack_search_channels` (by name or ID) | name, public/private, **archive status**. Archived → refuse (§7.3). | T2 |
| `thread` | `slack_read_thread` | channel + root `ts`, for the context panel (§7.10) | T3 |
| `whoami`, author names | `slack_read_user_profile` (current user or a user ID) | cached | T4 |

- **Parsing.** Tool results are text/JSON content blocks. Each tool has a typed parser in `providers::slack_mcp::schema`. Unknown fields are ignored. A missing required field is `ProviderError::Decode` naming the tool.
- **Result format (checked 2026-10-26).** https://docs.slack.dev/ai/slack-mcp-server/ documents tools and inputs but not result shapes, and the tools have no output schema. What the official server returns, from a live `slack_read_channel` read and a third-party client verified live against the server (github.com/scalekit-developers/workflow-agents-demos, `offer-letter-routing-agent/connectors/slack_mcp.py`):
  - every result is one `text` content block whose text is a **JSON string** (no `structuredContent`), with slashes escaped (`https:\/\/…`), so a literal `https://` search misses links;
  - `slack_send_message` → `{"message_context":{"message_ts":"<ts>","channel_id":"<channel>"}, …}`;
  - `slack_read_channel` → `{"messages":"<text>","pagination_info":"<text>"}`, where the messages text is `Channel: <name or DM> (<id>)` followed by blocks `=== Message from <Name> (<user ID>) at <time> ===`, a `Message TS: <ts>` line, and the message in Slack markup (links as `<url|label>`).
  The send parser therefore looks for, in order: a message link anywhere (escaped or not); a `message_ts`/`ts` key at any depth; a labelled ts in prose (`Message TS: …`). The channel comes from `channel_id`, else the request. Fixtures: `crates/pr-nexus-providers/tests/fixtures/slack-mcp-*.json` (generic data).
- **Recovery when the result is unreadable.** A successful `slack_send_message` whose result has no readable `ts` is never reported as failed. PR Nexus calls `slack_read_channel` on the target channel (newest 20 messages, `oldest` = now − 60 s), keeps messages by the signed-in user that match the sent text (every PR URL present, else the first line), and takes the newest. If that read fails or finds nothing, the send is `SentUnconfirmed` ("sent, link unknown"; §7.9). On a DM the read may be refused: PR Nexus does not request `im:history`, so DM sends with an unreadable result fall back to "sent, link unknown".
- **Attribution.** Slack appends `*Sent using* <@<app user>|<app name>>` ("Sent using PR Nexus", or the name of whatever app owns the client ID) to messages posted through MCP. PR Nexus cannot turn it off; the preview does not show it, and recovery matching ignores it.
- **Schema pin.** On connect, `tools/list` is compared with the pinned tool names and required input fields. A mismatch shows "Slack MCP changed; update PR Nexus" and disables writes for that workspace; reads degrade gracefully.
- **Errors.** MCP tool errors (`isError: true`) and JSON-RPC errors map to `ProviderError` (§4.3):
  - `ratelimited` or HTTP 429 → `RateLimited` (honour `Retry-After` when present, else back off 30 s);
  - `not_in_channel`, `channel_not_found` → `Api`;
  - 401, `invalid_auth`, `token_revoked` → `Unauthorized`;
  - `missing_scope` → `Forbidden`.
- **Throttling.** The client allows at most 1 write per second per channel and 1 in-flight call per tool per workspace. There is one MCP session per workspace, opened lazily and kept while the app runs.
- **Tools not used:**
  - `slack_send_message_draft`: the in-app preview replaces drafts (D13), and drafts have no `reply_broadcast`;
  - `slack_read_channel`: no history crawling (§1); it is used only to recover the `ts` of a message just sent (above), and is not pinned, so recovery degrades when it disappears;
  - `slack_add_reaction` and `slack_get_reactions`.

**Target picker (`TargetCombobox`).** The review dialog's target field and the bulk dialog's **All to one channel** field are one Select2-style combobox (ARIA `combobox` input with `aria-autocomplete=list`, `aria-activedescendant` and `aria-describedby` on the helper line, over a `listbox` of `option`s):
- **Order.** Suggestions first, under **Code owners and open requests**: the PR's open request targets and the mapped channels of its CODEOWNERS teams (`review_targets`), filtered by the typed term against the channel name and team. Then **Channels** and **People · direct message** from search, without repeating a suggestion. The bulk dialog has no suggestions.
- **Search.** Typing `#term` searches channels, `@term` searches people, a bare term searches both (`slack_search_targets(workspace, query)`, up to 10 of each). The call waits 250 ms after the last keystroke and results are cached per workspace and query while the dialog is open. Search runs only in the workspace picked in the field's workspace select (shown when more than one workspace is connected; defaults to the current target's, else the first signed-in one). When one kind fails and the other returns results, the results are shown; when nothing is found and a call failed, the list shows `Search failed: <reason>`.
- **Members.** Picking a person stores their user ID (`U…`/`W…`) as the target channel: `slack_send_message` delivers it as a direct message, the request record keeps the ID and the person's name, bumps reply in that DM thread, and the UI labels it `@Name`.
- **Keyboard.** ↑/↓ move the active option, Enter picks it, Esc closes the list without closing the dialog, Tab leaves. When not editing, the field shows the chosen target's label.
- **Fallback.** A term shaped like a Slack ID (`C`, `G`, `D`, `U` or `W` plus 8 or more letters and digits) adds **Use <ID>**. The helper line (`Type # to search channels or @ to search people; a person gets a direct message.`) ends with **Enter an ID**, which shows ID and name fields and a **Preview** button, for workspaces where search is unavailable.

**Multi-workspace (MUST, v1): any number of workspaces.**
- The user picks which workspaces to use by connecting them in onboarding or Settings → Accounts; no workspace is preconfigured. There is one Slack account per workspace (`accounts.slack_workspace` UNIQUE), each with its own OAuth grant and keychain tokens, and its own MCP session.
- The `ChatProvider` registry is keyed by workspace. Every write and read picks the provider from the mapping row's `slack_workspace`.
- Mapping rows carry `slack_workspace`. The channel picker and `slack_channels_cache` are scoped per workspace. Discovery search runs only in the target channel's workspace.
- The preview card shows `workspace · #channel` (e.g. `acme · #team-reviews`), or `workspace · DM` for a DM. Bulk grouping is by `(workspace, channel_id)`, so a batch can post to several workspaces in one Send, each with its own token.
- A workspace that is missing or unauthorized blocks only its own channels. The other workspaces still send, and the result report says "re-auth <workspace>".
- There is no default workspace setting. CSV rows without a workspace get the one picked in the import dialog (§7.5); the picker opens on the row's workspace, else the only or last-used connected workspace, else the first.

**Create your Slack app (link in wizard step 5 and Settings → Accounts).** No client ID ships with PR Nexus (D20). Instead, a **Create your Slack app** link opens Slack's create-from-manifest flow, prefilled with the manifest committed at `assets/slack-app-manifest.json`.
- **URL format** (https://docs.slack.dev/app-manifests/configuring-apps-with-app-manifests/, "Sharing manifests"): `https://api.slack.com/apps?new_app=1&manifest_json=<URL-encoded JSON>`. Slack also accepts `manifest_yaml=`; PR Nexus uses JSON.
- **Building the link:**
  - At runtime, PR Nexus loads the manifest (embedded with `include_str!`).
  - It replaces `oauth_config.redirect_urls` with the current redirect URI (the global setting, or the workspace override being edited).
  - It serializes the manifest compactly and percent-encodes every reserved character (`encodeURIComponent` semantics).
  - It opens the link in the system browser from Rust. The default link is about 930 characters long.
- **Manifest contents** (generic, no company data):
  - `display_information.name` = `PR Nexus`, with a one-line description;
  - `oauth_config.redirect_urls` = `["http://127.0.0.1:47821/mcp/oauth/callback"]`, the default of `slack.mcp.redirect_uri`;
  - `oauth_config.scopes.user` = the user scopes listed above, with `search:read.private` repeated in `user_optional` (optional scopes must also appear in `user`); no bot scopes and no bot user;
  - `oauth_config.pkce_enabled` = `false`, so the app stays confidential: enabling PKCE is one-way, so the user decides that in the app settings, not the manifest;
  - `settings`: `is_mcp_enabled: true` (the manifest form of App settings → Agents → "Slack Model Context Protocol (MCP) Server"), and `org_deploy_enabled`, `socket_mode_enabled`, and `token_rotation_enabled` all `false`;
  - `_metadata.major_version` = `1`.
- **Help text under the link** (wizard and Settings):
  > 1. Click **Create your Slack app**, pick the workspace, and confirm. Slack prefills name, scopes, redirect URL, and MCP access.
  > 2. In the new app, check **Agents → Slack Model Context Protocol (MCP) Server** is **On**. Turn it on if it is off.
  > 3. Copy the **Client ID** and **Client Secret** from **Basic Information** and paste them here. The secret is stored in your system keychain, never in PR Nexus settings.
  > 4. Install the app to the workspace. Slack lets only **internal** apps (created in your workspace, not distributed) or **Slack Marketplace** apps use MCP, and your workspace admin may need to approve it first.
  >
  > Already have an MCP-enabled Slack app approved in your org? Paste its client ID instead (and its secret, unless the app uses PKCE), and set the redirect URI to one registered on that app.
- **Several workspaces:** an internal app belongs to one workspace. To connect another workspace, create a second app there and put its client ID and secret in that workspace's override (§7.12). Enterprise Grid org-wide apps (`org_deploy_enabled`) are out of scope for the manifest.

**Fallback and future option: direct Web API (documented, not built in v1).** If a workspace cannot use the Slack MCP server (for example, its admins approve no MCP-enabled app), a `SlackWebApiChatProvider` can implement the same trait. It is the same `ChatKind::Slack`, selected by `accounts.auth_mode='slack_user_token'`, so the core, planner, and UI do not change.
- **Auth:** a user token (`xoxp-…`) from an internal Slack app the user creates in that workspace from a manifest, pasted into PR Nexus and checked with `auth.test`.
- **Methods:**
  - `chat.postMessage` (with `thread_ts` and `reply_broadcast` for bumps) and `chat.getPermalink`;
  - `conversations.list`, `conversations.info`, and `conversations.replies`;
  - `search.messages` (scope `search:read`) and `users.info`.
- **Wire escaping:** this path sends Slack mrkdwn, so it must escape `&`→`&amp;`, `<`→`&lt;`, `>`→`&gt;` in the outgoing text.
- **Rate-limit caveat:** since 2025-05-29, commercially distributed non-Marketplace apps are limited to 1 request/min on `conversations.history` and `conversations.replies`. Internal apps are exempt (https://docs.slack.dev/changelog/2025/05/29/rate-limit-changes-for-non-marketplace-apps/). That is why this path would use one internal app per workspace, never a shared distributed app. The caveat does not apply to the MCP path, which reads only threads through `slack_read_thread`.

### 7.13 Risks: Slack via MCP

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| R1 | **Reusing another app's client ID.** The user may point PR Nexus at an MCP-enabled Slack app their org already approved for another MCP client. | That app's name appears on the consent screen, in the workspace's connected apps and audit logs, and as the app shown on messages posted through it. PR Nexus depends on that app staying approved, on its registered redirect URI and port, and on its scopes. Its owner can change or revoke it at any time. Slack issues one user grant per app and user, so the two clients may share it. | Configurable: the client ID and redirect URI are settings, never in the repo. Remove workspace never revokes the grant. The recommended path is the user's own app from the **Create your Slack app** link (§7.11); switching only changes settings. |
| R2 | **Client authentication varies by app.** Apps created from the manifest are confidential and need the secret. A client ID alone works only for a PKCE-enabled app. Slack's acceptance of PKCE parameters alongside a secret on a non-PKCE app is not documented. | Sign-in fails at the code exchange. | M10 spike (§7.11): rmcp sends a public-client PKCE exchange without a secret and `client_secret_post` + PKCE with one, verified against mocks; the PKCE-only exchange is verified live against real Slack (2026-10-26); secret + PKCE on a confidential app is still to be confirmed live. First-class client secret field (keychain). Clear error naming the missing secret on `invalid_client`/`bad_client_secret`. Manual exchange fallback (§7.11), which can drop the PKCE parameters if Slack rejects them. |
| R3 | **MCP rate limits are only partly documented.** Search and send are "special". Limits are per app, so they are shared with the other client when an app is reused. | Discovery or sends get `ratelimited`. | Discovery is self-throttled to ≤10 calls/min per workspace with a 24 h cooldown (D15). Backoff on `RateLimited`. Background discovery pauses first; user-initiated sends keep priority. |
| R4 | **Loopback port collision.** A reused app has one registered redirect URI, so PR Nexus and the other client share the same port. | If both run a Slack sign-in at once, one cannot bind, or receives the other's callback. | Bind only during sign-in. Never fall back to another port. Clear port-in-use error. A callback with a foreign `state` is rejected and the flow keeps waiting. |
| R5 | **Tool schema drift.** The Slack MCP tools are not versioned. | Calls fail, or results parse wrong. | Pinned tool names and required fields, checked against `tools/list` on connect (writes disabled on mismatch). Contract tests replay recorded, scrubbed fixtures (§13). A manual check against the live server is on the pre-release checklist. |
| R6 | **Markdown conversion.** The send tool converts standard Markdown on the server. | Characters such as `*`, `_`, or `[` in PR titles get reformatted. | §7.6 wire rule and its M10 read-back check. |
| R7 | **Admin approval.** Without an approved MCP-enabled app, a workspace cannot connect. | No Slack features for that workspace. | Clear error on the consent failure. The Web API path above is the documented fallback. |

**Workspace routing (team ID).** Every Slack target, request and mapping is routed by the workspace's **team ID** (`T…`, from `auth.test` `team_id`), never by its domain.
- **Why:** on an Enterprise Grid, a channel link pasted from the browser carries the *org* host, and that host can equal another connected team's `auth.test` domain. Resolving by domain with a first match picked the wrong workspace, which answered `channel_not_found`. Slack documents `url` in `auth.test` as the workspace URL and mentions only `enterprise_id` for grids (https://docs.slack.dev/reference/methods/auth.test); that org-host links can collide with a team domain is not documented.
- **Storage:** migration 0006 adds `slack_team_id` to review requests and mappings, and drops the UNIQUE constraint on the account domain (two grid teams can share it). Requests are backfilled when their domain belongs to exactly one connected account. Mappings are not backfilled, since their domain may come from a pasted link; their first successful send pins the team. The disabled set and the priority order are settings keyed by team ID.
- **Resolution:** the team ID when set; else the one workspace whose domain matches; never a first match. A domain that matches several workspaces is ambiguous and goes through fallback.
- **CSV:** the `slack_workspace` column (and the import's workspace pick) accepts a domain or a team ID.
- **Enable switch:** a disabled workspace keeps its grant but is skipped for search, sends, bumps and status reads. Its existing requests show a "workspace disabled" note.
- **Priority:** the user orders workspaces in Settings → Slack (drag, or the up/down buttons). The order is saved.
- **Fallback:** a new request whose workspace is unknown or ambiguous probes the enabled workspaces in priority order with a read-only call (`slack_read_channel` with `limit: 1`, or the user profile for a person) before posting. It moves to the next workspace only on `channel_not_found`, `not_in_channel`, `user_not_found`, `invalid_auth` or `token_revoked`. A timeout, 5xx, rate limit or unknown error stops at once. The workspace that worked is saved on the target, the request and the matching mapping rows, and is named in the success toast and on the request row. Bumps always use the request's own workspace and never fall back.
- **Picker:** one search runs across every enabled workspace in priority order; results are merged, deduped by channel ID and labeled with the workspace name. The target chip names the workspace.

### 7.12 Config: everything org-specific is user data, not code

Nothing org-specific is hard-coded or shipped: GitHub orgs, Slack workspaces, teams, and channels are chosen by the user in onboarding and Settings, and stored in SQLite. The repo contains no company names or company data; examples and fixtures use neutral placeholders (`acme`, `team-a`, `#team-reviews`). The rule lives in AGENTS.md ("no org/workspace/channel literals in code, fixtures use placeholders") and is enforced in review.

| Key (`settings`) | Default | Example user value |
|---|---|---|
| `providers.vcs` / `providers.chat` | unset until wizard steps 1–2 | `"github"` / `"slack"` |
| `onboarding.state` | unset (wizard starts at step 1) | `{"step":"connect_chat","completed":["choose_vcs",…],"mapping_skipped":false,"slack_skipped":false,"finished_at":null}` |
| `github.tracked_orgs` | `[]`: none; onboarding asks the user to pick | `["acme","acme-labs"]` |
| `github.include_personal_repos` | `false` | false |
| `github.codeowners_paths` | `[".github/CODEOWNERS","CODEOWNERS","docs/CODEOWNERS"]` | same |
| `github.files_cap` | 3000 | same |
| `sync.focused_interval_s` / `unfocused_interval_s` | 180 / 900 | same |
| `sync.keep_closed_days` | 7 | same |
| `slack.workspaces` | none: derived from connected Slack accounts | `acme`, `acme-labs` |
| `slack.mcp.url` | `https://mcp.slack.com/mcp` | same |
| `slack.mcp.client_id` | unset: the wizard asks for it; no client ID ships with the app (D20). The client secret is not a setting: it lives in the keychain (§9). | `<client ID of the user's Slack app>` |
| `slack.mcp.redirect_uri` | `http://127.0.0.1:47821/mcp/oauth/callback`, the same value as in `assets/slack-app-manifest.json` (a unit test keeps them equal); must match a redirect URL registered on the app exactly | `http://127.0.0.1:<port>/mcp/oauth/callback` for an existing app |
| `slack.mcp.workspace_overrides` | `{}`: every workspace uses the global client | `{"T0000000":{"client_id":"<…>","redirect_uri":"http://127.0.0.1:<port>/mcp/oauth/callback"}}` (secret in the keychain under that client ID) |
| `review.requester` | `""` | `""` |
| `review.min_bump_interval_hours` | 24 | 24 |
| `review.blocked_review_required_is_ready` | `true` (§7.1 Δ, owner decision) | true |
| `review.discovery_search` | `true` (if `search:read.public` is granted) | true |
| `review.bulk_bump_max_threads` | 3 | 3 |
| `review.bump_broadcast` | `true`: a bump reply is also sent to the channel (`reply_broadcast`, same as the skill). Off: thread reply only. Settings → Review **Also post bumps to the channel**; the preview says which. | true |
| `review.also_request_github_team` | `false` (checkbox per preview) | false |
| `projects.ticket_key_regex` | `([A-Z][A-Z0-9]+-\d+)` | e.g. restrict to the user's Jira keys |
| mapping | empty; filled by CSV import, picker, corrections | — |

## 8. Projects & UI

### 8.1 Projects and auto-link rules

- A project is a local, user-defined group. It is the primary navigation; orgs are only a filter.
- Links live in `pr_projects` (many-to-many); a PR can belong to several projects. Rules are evaluated after every account sync over all of that account's PRs (a superset of "new or changed", cheap at this scale), and again after a rule is created, edited or deleted and after a Restore:
  1. Links with `source='manual'` or `'excluded'` are never touched by rules.
  2. For each active rule, ordered by `priority` desc, a match inserts `source='rule'`. This is skipped if an `excluded` row exists for the same (PR, project).
  3. Rule links whose rule no longer matches are removed. Manual links are kept.
- Rule kinds:
  - `TicketKeyInBranch`: a key extracted from `head_ref` (and title) with `projects.ticket_key_regex`, compared case-insensitively to the pattern (e.g. `PROJ-123` or `PROJ-*`). The extraction regex is also applied case-insensitively, so a branch such as `feat/proj-123-login` yields `PROJ-123`;
  - `BranchRegex`;
  - `Label` (exact);
  - `Repo` (`owner/name` glob);
  - `TitleRegex`.
- Regexes are compiled once, with the `regex` crate (no backtracking, so they are safe against untrusted titles), with a 1 MiB size limit and patterns of at most 256 characters. Saving an empty, too long or invalid pattern is rejected with an inline error. A stored rule that no longer compiles is skipped and counted, never fatal.
- When several rules of one project match, the link records the highest-priority rule (ties: lowest id). Deleting a rule removes its links, then relinking lets another matching rule take over.
- **Link semantics:**
  - Adding a PR by hand writes `manual`, which overrides a `rule` or `excluded` row.
  - Removing a plain manual link deletes it. Removing a rule link, or a manual link that a rule would re-create, writes `excluded` so the next sync does not bring it back.
  - **Restore** deletes the `excluded` row; the rules then decide again.
- **Manual links** (the spec allows no per-row buttons and no second bulk action, D16; so the least intrusive options were chosen, D21):
  - **Drag and drop:** drag a row onto a project in the sidebar or onto a project section header. Dragging a selected row drags the whole selection (distinct `PrId`s), including PRs picked with Select all that are not loaded; the drag image is an `N PRs` badge. The drop target highlights. The links are written all or nothing in one transaction, and a short confirmation (`Added 2 PRs to Billing`) appears with **Undo**. `link_prs` returns what each changed row was before (not linked, rule link with its rule, or excluded); Undo sends that back to `undo_link_prs`, which in one transaction restores each row that is still manual (a rule deleted in the meantime leaves no link) and leaves rows changed since alone.
  - **Project dialog → Add pull request:** a search field (title, repo, `#number`) listing up to 8 open PRs not yet linked; `Enter` adds the first. This is the keyboard path.
- **Project dialog** (section `⋯` → Edit auto-link rules):
  - Color swatches (`#rrggbb`).
  - Rules editor: one row per rule with kind, pattern, priority, Save and Delete, plus Add rule. A debounced (250 ms) live preview reads `Matches N open PRs`; it counts open, non-hidden PRs and shows the validation error instead when the pattern is invalid.
  - Linked pull requests: each with a `Manual`, `Rule` (tooltip: the rule) or `Excluded` badge and Remove or Restore.
- PRs with no project go to an **Unassigned** section, pinned last.

### 8.2 Views (wireframes)

**Visual style.** Dark, Linear-like, dense:
- compact rows (about 44 px, two lines: title, then `owner/repo · CI`);
- collapsible project sections with a count and a tri-state checkbox;
- a row checkbox on every PR;
- a **floating bulk action bar** at the bottom center that appears only while something is selected;
- muted text, one accent color (violet) for the primary action and the selection, status pills for review decision.
- **pending reviewers** next to the review pill: a stack of 20 px avatars of the requested users and teams that have not reviewed yet. Users first, then teams, each by login case-insensitively, so the order does not change between syncs. More than 3 draws the first 3 overlapped plus a `+N` chip. Teams without an avatar, and any avatar that fails to load, show initials (teams in a square, users in a circle). Hovering or focusing the stack shows the shared Tooltip with everyone: `Name (@login)` for users, `Team name (org/slug)` for teams. The names and avatar URLs come from the same `reviewRequests` nodes as the search fragment (cheap scalars, no extra query) and live in `pr_requested_reviewers.name` / `avatar_url` (migration 0003, nullable, so older rows show initials until the next sync). Every sync replaces a PR's requests, so a reviewer who submits a review drops off on the next sync. URLs carry an explicit size (`s=64`); the stack is never taller than a text line, so the row height does not change.

Approved mockups (AI-generated; style references, not pixel specs):
- [`docs/ui/list-view.jpg`](ui/list-view.jpg): list, sync footer, bulk bar, per-row request indicator (Slack icon + `Requested · 2d ago` / `Bumped · 5h ago` / dimmed `Not requested`; no action buttons on rows). Known differences, where the spec wins:
  - the **funnel filter button** next to the search bar is missing in the mockup but required;
  - project section headers need their own tri-state checkbox (the mockup only shows the collapse chevron);
  - the request indicator also carries the ↗ link to the original Slack message.
- [`docs/ui/slack-preview.jpg`](ui/slack-preview.jpg): the Slack preview modal only (cropped; in the app it sits centered over the dimmed list view). The rendered message is illustrative: the real text is the §7.6 template, byte-exact, and the sender shown is **you** (messages are posted with your user token), not an "APP" bot. Bumps appear in a separate "Bump · thread reply" group per channel.

**Layout.**
- **Sidebar** (navigation only, no persistent filters): My PRs, To review, then a **Projects** section listing active projects with a `+` (New project) button. With no projects it shows an empty state ("No projects yet…") with a **Create project** button; both open an inline name field (`Enter` creates via `create_project`: trimmed, non-empty, unique; `Esc` cancels) and select the new project. Settings is **not** a project entry. There is no org checkbox list here; orgs are a filter in the menu.
- **Sidebar footer:** Settings, then sync status plus a refresh button: `● Synced · 2m ago  ⟳`, `◌ Syncing…`, `▲ Rate limited · resets 14:32  ⟳`, or `▲ Sync failed · <reason>  ⟳` (§6). Clicking the text shows per-account detail.
- **Header:** view title and subtitle, the **search bar**, and a single small **filter icon button** (funnel). There is **no** overflow `⋯` button in the header.
- **Subtitle:** in My PRs and To review, `N open PRs across P projects`, where P counts the active projects (archived ones only when the Archived filter is on), plus `· U unassigned` when some shown PRs have no project. In a project view it reads `N open PRs in this project`. In a project view the title can be double-clicked to rename the project, like a section title.
- **Desktop shell:** the main window opens maximized on first launch; afterwards its size, position and maximized state are restored (`tauri-plugin-window-state`, never the visibility flag). Release builds (bundled frontend) suppress the WebView's native context menu (Reload, Inspect Element); dev builds keep it. Native file drops are disabled on the window so HTML5 row drag and drop works.
- **List header:** a global tri-state checkbox with `N of M selected` (or `M PRs` when nothing is selected).

**List (home)**:

```
┌──────────────────┬──────────────────────────────────────────────────────────────────────────┐
│ ◆ PR Nexus       │ Pull Requests              [🔍 Search projects, PRs, descriptions… ] [⏷] │
│                  │ 5 open PRs across 2 projects                                             │
│ ⌂ My PRs       ● │ ⊟ 2 of 5 selected                                                        │
│ ◷ To review      │ ┌ ▾ ⊟ Auth V2 (3) ────────────────────────────────────────────────── ⋯ ┐ │
│ ▤ PROJECTS     + │ │ ☑ #142 Migrate login to new auth +1 ✓Approved 💬0  ⧉ Bumped · 5h ago ↗│ │
│   ● Auth V2      │ │   acme/webapp · ● Passing                                            │ │
│   ● Billing      │ │ ☑ #138 Add passkeys support  ● Review pending 💬1  ⧉ Not requested  │ │
│                  │ │   acme/auth-service · ● Passing                                      │ │
│                  │ │ ☐ #135 Refresh token rotation ✕ Changes req. 💬3  ⧉ Not requested   │ │
│                  │ │   acme/auth-service · ● Failing                                      │ │
│                  │ └──────────────────────────────────────────────────────────────────────┘ │
│                  │ ▸ ⊟ Billing revamp (2) · collapsed                                    ⋯  │
│                  │ ┌ ▾ ☐ Unassigned (1) ──────────────────────────────────────────────────┐ │
│                  │ │ ☐ #23  Fix tax calculation   ● Review pending 💬0 ⧉ Requested · 5d ↗│ │
│                  │ │   acme/billing · ● Passing                                           │ │
│                  │ └──────────────────────────────────────────────────────────────────────┘ │
│ ⚙ Settings       │                                                                          │
│ ● Synced · 2m  ⟳ │      ┌ 2 selected │ [Request / bump on Slack] │ [✕ Clear] ┐              │
└──────────────────┴──────────────────────────────────────────────────────────────────────────┘
 ☐ none · ⊟ partly selected · ☑ all · 💬 unresolved threads · ⧉ Slack icon (grey = not requested) · ↗ original Slack request
 ⋯ project actions (auto-link rules) · double-click a project title to rename · no per-row buttons
```

**Project sections.**
- Header: collapse chevron, tri-state checkbox, project name, count, and (for real projects, not Unassigned) a small `⋯` menu at the right.
- **Empty projects** keep their section in Project grouping (`No PRs here yet. Drag PRs onto this project, use Add to project (p), or add an auto-link rule from the actions menu.`) so they can be renamed and receive drops. A search or any menu filter hides empty sections. A project view shows only its own empty section.
- **Rename:** double-click the project title to edit it inline. `Enter` saves (trimmed, non-empty, unique; otherwise an inline error and the editor stays open), `Esc` or clicking away cancels and restores the old name. It calls `rename_project`. Unassigned cannot be renamed.
- The section `⋯` menu holds **project-level actions only**, kept minimal: Edit auto-link rules (§8.1), Archive project, Delete project (with confirmation). It has no Rename entry (use double-click) and no Select all entry (use the checkbox).

**Selection: three levels, tri-state.**
- Levels: the **global** checkbox in the list header, one per **project section** header, one per **row**.
- Selection is a set of `PrId`s held in the view's frontend state; section and global states are derived, never stored.
- A parent shows ☑ when all its visible children are selected, ⊟ (indeterminate, `aria-checked="mixed"`) when some are, ☐ when none are.
- Toggling a parent: if it is ☑ it deselects all its visible children; otherwise (☐ or ⊟) it selects all of them.
- **Visible** means passing the current filters and search. The global checkbox covers all visible rows across sections, collapsed sections included. Rows hidden by a filter or search are never selected by a parent toggle. Rows already selected that a later filter hides stay selected; the bar then reads `3 selected (1 hidden)`, and `Clear` drops them too.
- **Identity, not position:** a PR in several projects appears in several sections. Checking it in one section checks it in all of them, and every section containing it recomputes its state (a collapsed section can show ⊟ because of it). Counts and bulk actions use distinct `PrId`s, so the bar's `N selected` and the preview never double-count.
- Selection is per view and session-only (not saved in `view_state`); switching views keeps each view's selection until the app quits.

**Add to project.** The selection bar has **Add to project** (folder-plus icon; `p`, which acts on the focused row when nothing is selected). It opens a menu above the bar: the non-archived projects with their color dots, a separator, then **New project from selection…**, which turns the menu into a name field (`Enter to create with N PRs · Esc to go back`; same trimmed, non-empty, unique rule as rename, with the error inline). Picking a project calls `link_prs`; creating calls `create_project_with_prs`, which inserts the project and its manual links in one transaction, so an unknown PR id leaves no empty project behind. Either way a toast confirms (`Added 2 PRs to Billing`, `Created Launch with 2 PRs`) and the sidebar scrolls to the project and highlights it for 1.5 s. Arrow keys move through the menu, `Esc` steps back from the field and then closes it, and a click outside closes it. Switching views or clearing the selection closes it. The sidebar's **+** field shows **Add N selected PRs** (checked by default) while the list has a selection, and then also uses `create_project_with_prs` before opening the new project.

**Icons and tooltips.** Buttons pair a 16 px line icon (`app/src/icons.tsx`) with their label: the Slack mark on Request / bump on Slack, the GitHub mark on Open on GitHub and Connect, ✕ on Clear, folder-plus on Add to project, chevrons on the wizard's Back and Next, and so on. Dialog, banner and toast close buttons are icon-only ✕ (`CloseButton`, accessible name `Close` or what it dismisses). Every icon-only button, and the selection-bar buttons, have a shared `Tooltip` instead of a native `title`: it shows after 350 ms of hover or at once on keyboard focus (not when a click or a dialog opening moves focus), hides on leave, blur or `Esc`, gives the label, the shortcut if any and a one-line explanation, and is linked with `aria-describedby`.

**Text fields.** Every text input and textarea turns off spellcheck, autocorrect, autocapitalize and autocomplete (`plainText` in `app/src/textInput.ts`), set per element because WKWebView does not reliably inherit them. A free-text Slack message `<textarea>` may keep spellcheck (marked `data-prose`) but never autocorrect; today there is none (previews are read-only). `text-fields.test.ts` fails on a field without the spread or one that overrides it after the spread. As a safety net, `guardTextFields(document)` (started in `main.tsx`) applies the same attributes to any text field the spread missed, including fields added later or changed afterwards.

**Rows have no action buttons.** A row is selected only with its checkbox (or `x`). The **only** Slack action in the list is the floating bar's **Request / bump on Slack**, which decides request vs bump per PR and per channel from `review_requests` (§7.7, §7.9).

Row content, each part toggled under **Show**: number and title; `owner/repo`; CI (colored dot + label); review-decision pill (`✓ Approved`, `● Review pending`, `✕ Changes requested`); 💬 unresolved thread count (default on; icon + number, tooltip and accessible label `N unresolved of M review threads`, dimmed at 0; never a bare `0/0`); owning teams; last activity; project badges (`+1` when in several projects); the **Slack request indicator**. Layout matches the mockup: title and repo on the left, then aligned review, threads, and Slack columns.

**Slack request indicator** (from the newest active `review_requests` row):
- Slack icon plus the newest event and its age: `Requested · 2d ago`, or `Bumped · 5h ago` once `last_bumped_at` is set. Tooltip: `workspace · #channel`, the request age, and the bump count. With several active channels: `Requested 2d ago · 2 channels`, with a hover list.
- A small **link icon ↗** opens the **original request message/thread**. It tries the Slack app first with `slack://channel?team=<team_id>&id=<channel_id>&message=<root_ts>`, where `team_id` comes from the Slack account's `external_id`. If no handler is registered (`tauri-plugin-opener` errors), it opens the stored https `permalink` in the browser. Verify the deep-link format in M10; if Slack ignores `message`, use the permalink directly, since the desktop app intercepts Slack https links when installed.
- Rows without an active request show a greyed Slack icon and `Not requested` (as in the mockup), with no ↗. Show → "Not requested label" turns it off.

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
  - Archived and closed: show PRs in archived projects and merged/closed PRs still within `sync.keep_closed_days`. The header counts them apart (`3 open PRs across 2 projects · 1 merged or closed`).
  **Reset** clears filters only, not grouping, ordering, or Show.
- The funnel is an icon button immediately right of the search field. It shows a dot when any filter is active, including a quick filter. The menu closes on `Esc` and on a click outside it. The only other persistent filter UI is the quick filter bar below.
- **Quick filters:** a row of icon buttons at the right of the Select all bar, each with a tooltip and a count badge. Only one is on at a time; clicking it again turns it off, and Reset clears it too. Definitions (`quick_matches` in `listing.rs`, mirrored in `model.ts`):
  - **Draft:** the PR is a draft.
  - **No review requested:** not a draft, no reviewer or team requested on GitHub, no active Slack request, and no approval or change request yet.
  - **CI errors or conflicts:** failing checks (`failure` or `error`) or merge conflicts (`mergeable = CONFLICTING` or merge state `DIRTY`).
  - **Review declined:** changes requested.
  - **Ready to merge:** not a draft, approved, checks green, and no conflicts.
  The filter runs in `list_page` after the menu filters and search, so paging, section counts, Select all, the section checkboxes and drag all use the filtered result. `list_page` also returns `quick` counts per filter over the menu-filtered, searched rows before the quick filter, so each badge shows what clicking it would leave. The choice is saved with the view's filters (`filters.quick`).
- **Persistence:** grouping, ordering, Show, and filters are saved **per view** in `view_state` (`mine`, `to_review`, `project:<id>`) and restored on launch.

**Search bar:** always visible. Searches the whole view in the backend as you type (200 ms debounce), case- and accent-insensitive, whitespace-separated terms ANDed. A term matches the PR title, description (`bodyText`), `owner/repo`, number (`#142` or `142`), author, branch, or the name of a project the PR is linked to. A term is a plain substring, so punctuation needs no escaping: `service-ssp`, `acme/service-ssp`, `ssp`, `fix(api):`, `v1.2` and `#123` match as typed, and `123` or `#123` also match `#1234`. Each PR stores its folded text in `pull_requests.search_text` (NFKD, marks stripped, dash variants such as U+2011 turned into `-`, lowercase; migration 0002, refolded by migration 0004), and `list_page` matches it with `instr` per term. That takes under 20 ms over 5,000 PRs, so there is no FTS5 index. Search stays inside the current view and its open/archived scope. When that leaves nothing, the empty state says where the query does match (`Matches elsewhere: 2 in To review, 1 merged, closed or archived.`) with a button per way out: **Show in To review (2)** switches view and keeps the query, **Include archived and closed (1)** turns that filter on, and **Reset filters (n)** counts the matches the filters hide. While a search is in flight the header shows `Searching…`; a response for an older query is dropped. Results use the same paging, counts, grouping and selection as the unfiltered list. It combines with the menu filters. `/` focuses it, and `Esc` clears it.

**Loading and paging.** The window opens on the local database: `bootstrap` reads only stored state (no `gh` call, no network), and `finish_onboarding` returns at once and starts the first sync in the background. The list never waits for a sync; the sidebar shows `Syncing…` and the list refetches when it ends (polled every 3 s while a sync runs).
- `list_page` returns one page of the flattened list: section headers and rows in display order, about 50 entries per page, with an opaque cursor. Collapsed sections send only their header.
- Every page also carries what the header and selection need for the **whole** result, loaded or not: each section's PR ids and count, `N open PRs`, `U unassigned`, and the org choices with counts. So counts, the section and global checkboxes, select-all, the bulk bar, **Add to project** and drag and drop act on every matching PR, not just the rows on screen. Dragging a selected row moves the whole selection; `link_prs` writes all links in one transaction. Rows the UI needs but has not loaded come from `rows_by_id`.
- More pages load as a sentinel row nears the viewport (`IntersectionObserver`, 600 px ahead, with a `Loading more…` row); `j`/`k` past the last loaded row loads the next page too.
- When a sync or a write changes the data, the list refetches from the top with the number of rows already loaded, so the scroll position stays put.
- There is no cap on the number of PRs listed, and none on the GitHub search (§6). Sync stores each search page as it arrives; while it runs, the list header adds "Syncing… N so far", updated at most every 500 ms.
- The list query reads no PR bodies and loads labels and reviewers in one query each, not one per PR.

Keyboard: `j/k` move, `x` toggle select, `Shift+x` toggle the focused row's section, `Cmd/Ctrl+A` toggle the global checkbox, `r` open the Slack preview for the selection (or the focused row if nothing is selected), `s` open the focused row's Slack request, `o` open in browser, `f` open the filter menu, `p` open Add to project for the selection (or the focused row), `Enter` open the focused row's details, `/` focus the search (`Esc` there clears it), `Esc` clear the selection (also from a focused row checkbox). Single-key shortcuts ignore Cmd, Ctrl and Alt, so Cmd+R and similar keep their system meaning, and do nothing while typing or while a modal is open. The focused row is marked `aria-current` and scrolled into view.

**PR detail** (a slide-over on the right, not a modal; `bun run screenshot -- --detail <file>` renders it with mock data):

```
┌ list (stays interactive) ──────────┬ [Open] acme/repo-a · main ← feat/retry  [S][GH][×] ┐
│ #412 Add retry budget    ◀ open    │ #412 Add retry budget                              │
│ #409 …                             │ ┌Checks─┐┌Comments┐┌Approvals┐┌Mergeable┐          │
│                                    │ │ 4/6 ▬ ││ 11     ││ 1/2 (KL)││ Clean   │          │
│                                    │ └1 fail─┘└4 · 7───┘└────────-┘└─────────┘          │
│                                    │ ✕ You can't merge: a reviewer requested changes.   │
│                                    │   [Enable auto-merge][Squash ▾]                    │
│                                    │ Reviewers (KL)✓ (SR)✕ (Web)…   Participants ●●●+2  │
│                                    │ Slack: #team-a-reviews · requested 3d · Open ↗     │
│                                    │   bob: looking tomorrow                            │
│                                    │ ▸ Files changed (37)            with code owners   │
│                                    │ ▸ Details                                          │
│                                    │ ▸ Slack threads and mentions                       │
└────────────────────────────────────┴────────────────────────────────────────────────────┘
```

- The panel takes 40% of the window by default; its left edge drags (or takes ←/→ when focused) between 28% and 70%, kept per device. Clicking another row switches it; `j`/`k` and ↑/↓ move between PRs while it is open; `Esc` or ✕ closes it.
- Header: number, title, `repo · base ← head`, an Open/Draft/Merged/Closed pill, and icon buttons with tooltips: Request on Slack (`r`), Open on GitHub (`o`), Close.
- Cards: **Checks** passed/total with a bar (click lists failing and running checks with log links), **Comments** (on the PR · in code), **Approvals** approved/required with approver avatars (`a/?` when the requirement is unreadable, §7.2), **Mergeable**. Below them, a sentence saying whether the viewer can merge and why. "Can merge" is derived (permission, draft, conflicts, `mergeStateStatus`, review decision, required checks); GitHub has no exact flag for it.
- Auto-merge: enable with a method the repository allows (squash by default) or disable, through GraphQL `enablePullRequestAutoMerge` / `disablePullRequestAutoMerge`; the current state is shown, unavailable controls are disabled with a tooltip giving the reason, and the result is a toast. GitHub refuses auto-merge on a PR that is already mergeable (`CLEAN`); the panel says a direct merge is possible instead.
- People: requested reviewers (users and teams) as avatars with a status ring; participants stacked, at most 6, then `+N` with the rest in a tooltip.
- Slack: "Slack: no request yet · Request" when there is none; otherwise the latest request's root message and its last two replies. The full §7.10 view stays available as a collapsed section.
- Collapsed sections: **Files changed (n)** with their code-owner teams, and **Details**, the ownership and readiness reasons as plain sentences.
- Data: `pr_overview(id, refresh)` returns the cached copy (`pr_overview_cache`) first, then GitHub's; the panel refetches after each sync. A skeleton shows until the first copy arrives.

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
- Accounts. It starts with a **Providers** row showing the chosen git and communication providers (from the registry: GitHub and Slack only, so there is nothing to switch to in v1; no other provider is mentioned). Then: the GitHub account (re-connect with any of the three auth modes) plus the **tracked orgs picker**: checkboxes from `viewer.organizations` and orgs seen in search, and a "personal repos" toggle; Slack with the MCP client settings (the **Create your Slack app** link and its help text, client ID, client secret stored in the keychain, redirect URI; §7.11), one row per connected workspace, each with its own status, re-auth, remove, and an optional client override, and an "Add workspace" button);
- Mappings (table, import/export, dry-run diff; this is where a mapping skipped in the wizard is done later);
- Review (the config keys in §7.12);
- Sync;
- About (version, log folder, DB path, and **Re-run setup**, which reopens the wizard at step 1 with current values pre-filled).

### 8.3 Setup wizard (first run)

Onboarding is a **wizard of separate screens, shown one step at a time**, not a single settings page. It opens on first launch, and on every launch until it is finished.

**Steps** (v1, from `ProviderRegistry::setup_steps(GitHub, Slack)`, §5.1):

| # | Step | Content | Next is enabled when |
|---|---|---|---|
| 1 | Choose git provider | A single **GitHub** card, preselected. No other provider is shown. | a provider is selected (GitHub preselected) |
| 2 | Choose communication provider | A single **Slack** card, preselected. No other provider is shown. | a provider is selected (Slack preselected) |
| 3 | Connect GitHub | The three auth modes of §9 in order: `gh` CLI token (preselected when `gh auth status` succeeds), PAT, device flow. Validates with `viewer` and shows the login and scope check; SSO errors show the authorization link. | the account is verified |
| 4 | Choose orgs to track | Checkboxes for every org the user belongs to (§5 `list_organizations`: GraphQL, REST `/user/orgs`, orgs seen in search; SSO-blocked orgs carry a hint), a filter box, and the **Include my personal repos** toggle. Writes `github.tracked_orgs` and `github.include_personal_repos`. | at least one org is checked or personal repos is on |
| 5 | Connect Slack workspace(s) | The **Create your Slack app** link with its help text (§7.11), then the Slack MCP client fields: client ID (required once, empty by default: no client ID ships), client secret (optional, saved to the keychain; needed for apps created from the link; shows "saved in the keychain" once stored), and redirect URI (pre-filled with the default that the manifest registers, editable: it must match a redirect URL on the app). Below them come the list of connected workspaces (team name, `domain.slack.com`, `@user`, "signed out" when the tokens are gone, Remove), then **Sign in with Slack** for **one workspace at a time**. Sign-in saves the fields first, then opens the system browser; while waiting the button reads "Waiting for Slack…" with **Cancel**, with a countdown ("m:ss left", then "Still waiting for Slack."); the 10-minute timeout shows "Sign-in timed out, click to retry.", and denial and port-in-use errors are shown inline. Once one workspace is connected, the button becomes **Add another workspace**. | at least one workspace is signed in; otherwise Skip |
| 6 | Team → channel mapping | CSV import (pasted, or **Choose file…** in the native file dialog) with the default-workspace field and dry-run preview (§7.5), or **Skip**. The screen explains each column and shows an example with made-up data (`github_org,github_team_slug,github_team_name,slack_channel_name,slack_channel_id`, optional `slack_workspace`). A note says it can be done later in Settings → Mappings or from the preview's "Change channel". | always: the primary button reads **Import & next** when CSV is pasted, otherwise **Next** (continues without a mapping), or Skip |
| 7 | Done | Summary (account, orgs, workspaces, mapping rows or "skipped") and the **first sync** starting, with live progress. If the first sync fails, the status line shows `Sync failed · <reason>` with a **Retry sync** button that runs it again; Open PR Nexus still works, and the sidebar refresh button (§8.2) can retry later. "Open PR Nexus" goes to My PRs. | — |

**Navigation and rules.**
- A progress indicator at the top (one dot per step, `Step k of N` and the step name). `N` comes from the registry, so it changes if other providers add steps.
- **Back** and **Next** in the footer. Back is disabled on step 1. Back never undoes work already saved (a connected account stays connected); it only moves the screen. Going back to step 1 or 2 and changing the provider would drop later steps, but in v1 there is only one available provider per kind.
- **Steps 5 (Connect Slack) and 6 (mapping) are skippable.** Step 5 stays skippable after Slack sign-in shipped (M10), so the app is usable while a workspace admin has not approved the Slack app yet (§7.13 R7). Skip on that step records `slack_skipped` and does not pretend a workspace is connected; the same sign-in is in Settings → Slack. Steps 1–4 never show a Skip button.
- A disabled Next always shows a one-line reason above the footer (e.g. “Connect GitHub above to continue.”).
- Skip or Next on step 6 always lands on step 7 (Done); “Open PR Nexus” enables once the first sync settles and the summary has been visible briefly, so a double click on step 6 cannot skip the summary.
- Next writes the step's result immediately (settings, accounts, keychain), so each step is durable on its own.
- **Resumable:** `onboarding.state` (§7.12) stores the current step, completed steps, and `mapping_skipped`. It is updated on every Next/Back. If the app is closed mid-wizard, the next launch reopens the wizard at that step with saved values pre-filled. On launch, completed steps are re-validated cheaply (account still verifiable, keychain entry present); if one fails, the wizard resumes at the first invalid step.
- Step 7 sets `finished_at` and starts the first sync. After that, the app opens on the list, and the wizard is only reachable through Settings → About → Re-run setup.
- The wizard is a full window without the sidebar. It uses the same dark style as the list (§8.2).
- If mapping was skipped, the list shows a one-line dismissible hint ("No team → channel mapping yet · Import CSV") and the Slack preview resolves channels with the picker.

**Wireframes.** Step 1 (the shell is the same for every step):

```
┌ PR Nexus setup ────────────────────────────────────────────────────────────┐
│ ● ○ ○ ○ ○ ○ ○   Step 1 of 7 · Git provider                                 │
│                                                                            │
│ Where do your pull requests live?                                          │
│                                                                            │
│ ┌──────────────────┐                                                       │
│ │ ◉ GitHub         │                                                       │
│ │   github.com     │                                                       │
│ └──────────────────┘                                                       │
│   selected                                                                 │
├────────────────────────────────────────────────────────────────────────────┤
│                                                        [Back]  [Next →]    │
└────────────────────────────────────────────────────────────────────────────┘
```

Step 2:

```
┌ PR Nexus setup ────────────────────────────────────────────────────────────┐
│ ● ● ○ ○ ○ ○ ○   Step 2 of 7 · Communication provider                       │
│                                                                            │
│ Where do you ask for reviews?                                              │
│                                                                            │
│ ┌──────────────────┐                                                       │
│ │ ◉ Slack          │                                                       │
│ │ 1+ workspaces    │                                                       │
│ └──────────────────┘                                                       │
├────────────────────────────────────────────────────────────────────────────┤
│                                                      [← Back]  [Next →]    │
└────────────────────────────────────────────────────────────────────────────┘
```

Step 3:

```
┌ PR Nexus setup ────────────────────────────────────────────────────────────┐
│ ● ● ● ○ ○ ○ ○   Step 3 of 7 · Connect GitHub                               │
│                                                                            │
│ ◉ Use the gh CLI token   ✓ gh found · logged in as @you (recommended)      │
│ ○ Paste a personal access token   [ ghp_… / github_pat_…        ]          │
│ ○ Sign in with GitHub (device flow)   code: ABCD-1234  [Copy & open]       │
│                                                                            │
│ ✓ Connected as @you · scopes ok          (SSO link shown on 403-SSO)       │
├────────────────────────────────────────────────────────────────────────────┤
│                                                      [← Back]  [Next →]    │
└────────────────────────────────────────────────────────────────────────────┘
```

Step 4:

```
┌ PR Nexus setup ────────────────────────────────────────────────────────────┐
│ ● ● ● ● ○ ○ ○   Step 4 of 7 · Orgs to track                                │
│                                                                            │
│ Pick the GitHub orgs whose PRs PR Nexus should show.                       │
│ [Filter orgs…    ]                                                         │
│ ☑ acme          (member)                                                   │
│ ☑ acme-labs     (member)                                                   │
│ ☐ other-org     (seen in search)                                           │
│ ☐ Include my personal repos (@you/*)                                       │
├────────────────────────────────────────────────────────────────────────────┤
│ At least one org or personal repos required    [← Back]  [Next →]          │
└────────────────────────────────────────────────────────────────────────────┘
```

Step 5:

```
┌ PR Nexus setup ────────────────────────────────────────────────────────────┐
│ ● ● ● ● ● ○ ○   Step 5 of 7 · Connect Slack                                │
│                                                                            │
│ Your Slack app (no app ships with PR Nexus):                               │
│   [Create your Slack app ↗]  prefilled: name, scopes, redirect, MCP on     │
│   Then check Agents → Slack MCP Server is On, install the app (an          │
│   admin may need to approve it), and copy the credentials from Basic       │
│   Information. Or paste the ID of an MCP app your org already approved.    │
│   Client ID      [ ________________________ ]                              │
│   Client secret  [ ________________________ ]  optional · keychain         │
│   Redirect URI   [ http://127.0.0.1:47821/mcp/oauth/callback ]             │
│                                                                            │
│ Connected workspaces:                                                      │
│   ✓ acme · @you                                         [Remove]           │
│                                                                            │
│ Add a workspace (one at a time):                                           │
│   [Sign in with Slack]  opens the browser; pick one workspace there.       │
│   Waiting for the browser…  [Cancel]                                       │
│ [+ Add another workspace]                                                  │
├────────────────────────────────────────────────────────────────────────────┤
│ At least one workspace required                [← Back]  [Next →]          │
└────────────────────────────────────────────────────────────────────────────┘
```

Step 6 (the only one with Skip):

```
┌ PR Nexus setup ────────────────────────────────────────────────────────────┐
│ ● ● ● ● ● ● ○   Step 6 of 7 · Team → channel mapping (optional)            │
│                                                                            │
│ Tell PR Nexus which Slack channel each GitHub team reviews in.             │
│ [Choose file…]  Target workspace for rows without one: [acme ▾]            │
│                                                                            │
│ Preview: 42 rows · 40 new · 2 invalid (shown below)                        │
│ You can also map channels later in Settings → Mappings, or from            │
│ the Slack preview's 'Change channel'.                                      │
├────────────────────────────────────────────────────────────────────────────┤
│                                        [← Back]  [Skip]  [Import & next →] │
└────────────────────────────────────────────────────────────────────────────┘
```

Step 7:

```
┌ PR Nexus setup ────────────────────────────────────────────────────────────┐
│ ● ● ● ● ● ● ●   Step 7 of 7 · Done                                         │
│                                                                            │
│ GitHub: @you · 2 orgs · personal repos off                                 │
│ Slack: 1 workspace (acme) · mapping: 40 rows (or 'skipped')                │
│                                                                            │
│ ◌ First sync… 14 PRs found                                                 │
├────────────────────────────────────────────────────────────────────────────┤
│                                               [← Back]  [Open PR Nexus →]  │
└────────────────────────────────────────────────────────────────────────────┘
```

## 9. Auth, secrets & security

**GitHub auth**, in the order offered:
1. **`gh` CLI token** (default if `gh auth status` succeeds). Run `gh auth token --hostname github.com` on demand, with a 5 s timeout, and keep the result in memory only. It is **never stored**. It inherits the user's SSO authorizations, which is usually the least friction in SAML orgs.
2. **Fine-grained or classic PAT**, pasted by the user and stored in the keychain.
   - Classic: `repo`, `read:org`.
   - Fine-grained: per org, read access to Pull requests, Contents (for CODEOWNERS), Metadata, Checks/Commit statuses; Members (org) read for team membership; Pull requests write only for the M10 option.
   - An org may block classic or fine-grained PATs. That is why the `gh` token is the default (D1).
3. **OAuth device flow.** The client id is not committed (same rule as the Slack client id). It is read from `github.oauth_client_id` or `PR_NEXUS_GITHUB_OAUTH_CLIENT_ID`. When both are empty the wizard still shows the mode and explains that a client id is required; `gh` and a PAT stay usable. A token obtained this way is stored in the keychain. An org may need to approve the app.

Detect `X-GitHub-SSO: required; url=…` and show the authorization link.

**Slack auth:** OAuth 2.1 authorization code + PKCE against the Slack MCP server, with a user-configured client ID, an optional client secret (keychain), and a loopback redirect URI (§7.11).
- The authorize URL is opened from Rust, not from the webview.
- The callback listener binds `127.0.0.1` on the configured port only while a sign-in is running. It accepts one callback with a matching `state`.
- The PKCE verifier and the code never leave memory.

**Secrets**:
- Use `keyring` with service `dev.pr-nexus` and **one single entry**, `vault`. It holds a JSON map `{ "<kind>:<external_id>": <secret> }`, e.g. `github:<node id>` → token, one `slack:<team_id>:<user_id>` per workspace → `{access_token, refresh_token?, expires_at?}`, and one `slack-client:<client_id>` per configured Slack app → `{client_secret}`, entered before the first sign-in and shared by the workspaces that use that client. The app reads it once at startup and keeps it in memory (`secrecy::SecretString`).
- **Why one entry:** builds are unsigned (see the caveat below). After each update, macOS asks once per keychain item whether to allow access. With 1 GitHub account and N Slack workspaces, separate items would mean 1+N prompts per update; one entry means one.
- Never write secrets to SQLite, logs, or the frontend. Commands return only `has_token: bool`.
- **Keychain access never blocks and a pending prompt is not a failure.** Every keychain call runs on one dedicated background thread, one call at a time, so at most one system prompt is on screen. Startup returns at once; the window shows a calm hint, "Waiting for keychain access…", while the first read is pending (on macOS, the user may be reading the prompt). Code that needs a token waits for that read for up to **5 minutes**; display paths never wait. The UI polls `keyring_status` (`ready`, `waiting`, `timed_out`, `denied`, `no_backend`, `error`, `disabled`) and shows:
  - **Timed out** (no answer after 5 minutes): tokens stay in memory, and the read keeps going. When the keychain answers later, its entries are loaded and anything entered in the meantime is merged in and saved. **Retry** asks again.
  - **Denied** (Deny or Cancel on the prompt, or a dismissed secret-service prompt): "Keychain access was denied…", tokens in memory for this session, **Retry** asks again. After any failure the vault makes no keychain call on its own: saving a token keeps it in memory rather than reading or writing the keychain again, so a Deny is never followed by another prompt. Only **Retry** or a restart asks again, and tokens saved meanwhile are written once access is granted.
  - **No keychain** (no secret service, e.g. Linux CI) or **error**: the reason, tokens in memory, **Retry**.
  - **Disabled** (`PR_NEXUS_DISABLE_KEYRING=1`): tokens in memory, no Retry.
  Nothing is written to the keychain before it has been read once, so a failed or pending read can never overwrite the stored entry. A failed write keeps the tokens in memory and is retried. When the state turns `ready` the window reloads its bootstrap data. The process never crashes over the keychain. `PR_NEXUS_GITHUB_TOKEN`, when set, is the GitHub token for that process: it is preferred over `gh` and the keychain, and it is never written to the vault or to SQLite. `PR_NEXUS_DB_PATH` points the store at a chosen file (a temp database in development).

**Webview hardening**:
- Tauri v2 capabilities give the main window only `core:default`, `opener:allow-open-url` limited to `https://github.com/*`, `https://*.slack.com/*`, and `slack://*` (request deep links, §8.2), `dialog:allow-open` (the mapping CSV picker, §7.5; Rust reads the picked file), and the app's own commands.
- No shell or fs plugins in the webview.
- CSP: `default-src 'self'; img-src 'self' https://avatars.githubusercontent.com data:; connect-src ipc: http://ipc.localhost`. All network access happens in Rust.
- Message text, titles, and Slack replies render as text, never as HTML.

**Logs**:
- Use `tracing` with daily rotating files in the app log dir, kept for 7 days.
- A redaction layer masks `gh[pousr]_…`, `github_pat_…`, `xox[abeposr]-…` (including `xoxe-` refresh tokens), OAuth `code`, `code_verifier`, and `client_secret` parameters, and `Authorization` headers.
- Message bodies log at debug level only.

**macOS distribution without an Apple Developer ID (owner decision: v1 is unsigned/ad-hoc).**
- *Gatekeeper.* The DMG is ad-hoc sealed (`codesign --force --deep --sign -`), not notarized. A browser download gets the quarantine flag, and macOS refuses the first launch ("cannot verify the developer" or "damaged"). Workarounds, documented in README and CONTRIBUTING:
  1. **Preferred:** `gh release download vX.Y.Z -R harnesstap/pr-nexus -p '*.dmg'`. The repo is private, so `gh` is the natural path anyway, and files fetched by `gh`/`curl` carry no quarantine flag.
  2. Browser download: try to open the app once, then System Settings → Privacy & Security → **Open Anyway**. Control-click → Open no longer bypasses this on macOS 15+.
  3. Or `xattr -dr com.apple.quarantine /Applications/PR Nexus.app`.
- *Keychain re-prompt after every update.* An ad-hoc signature has no stable team identity, so the keychain's "Always Allow" rule is tied to the exact binary hash. Each new version triggers a "PR Nexus wants to use your confidential information stored in 'dev.pr-nexus'" prompt, which needs the login password and then **Always Allow**. The single `vault` entry above keeps this to **one prompt per update**. The About tab and release notes explain it. Denying the prompt is handled: accounts show "token unavailable — unlock", with a retry button. Nothing is wiped.
- *Future, optional:* Developer ID signing + notarization (Tauri reads `APPLE_*` secrets) would remove both issues. Nothing in v1 depends on it.

**Local MCP server for agents (Settings → Agents (MCP)).**
- *Off by default.* When enabled, the app serves MCP over Streamable HTTP at `http://127.0.0.1:{port}/mcp` (`rmcp`), on any free port or a fixed one from 1024 up. Toggling or changing the port starts, stops or moves the server without a restart; a taken port shows as an error in Settings and nothing else fails.
- *Auth.* Every request needs `Authorization: Bearer pnx_…` (256 random bits, kept in the `vault` under `mcp.token`, compared in constant time). Settings shows it masked, copies it, and regenerates it after a confirmation; the old token stops working at once.
- *DNS rebinding.* The Host header must be `127.0.0.1:{port}` or `localhost:{port}`, and an Origin header, when present, must be that same loopback origin. Anything else is a 403 before MCP runs.
- *Capabilities.* Reads, Local writes and Slack sends are separate switches. Tools are listed and allowed only when their capability is on; a refused call is a tool error. Every call goes through one audit hook (tool, capability, outcome, duration, client name; argument values are not logged).
- *Tools (Reads).* `list_projects`, `get_project`, `list_prs` (the list filter: statuses, starred, project, repo, author, state, query; 50 per page, up to 200, cursor paging) and `get_pr` (note, star, Slack request, cached branch status). They call the same services as the UI commands.
- *Next.* Local writes (projects, links, notes, stars) with a persisted audit log, then Slack sends behind a mandatory preview token, then a stdio proxy for clients without HTTP.

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
- Internal roadmap only: none of these is shown or promised in the app, README, or release notes. Implementing one means adding its registry descriptor (§5.1), a factory, and its setup steps; until then the UI shows GitHub and Slack only.
- GitLab and Gitea (`VcsProvider`): CODEOWNERS equivalents exist (GitLab `CODEOWNERS` with sections), so `decide_owners` stays as is, and new inputs go through `OwnershipInputs`.
- Slack direct Web API transport (`SlackWebApiChatProvider`), the documented fallback in §7.11, if a workspace cannot use the Slack MCP server.
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
| 6-target desktop matrix | **Change** | v1 **ships macOS arm64 DMG only**. Linux (self-hosted) and Windows (windows-latest, tags and dispatch only) **build** in CI (`cargo check` + `tauri build --no-bundle`) but are not attached to releases. Add Intel macOS and Linux/Windows bundles when someone asks. |
| Unsigned + ad-hoc reseal | **Copy (owner decision: no Apple Developer ID)** | The ad-hoc reseal is needed for arm64 Gatekeeper. Consequences and workarounds (Open Anyway / `gh release download`, one keychain re-prompt per update) are in §9. Developer ID signing + notarization is optional future work. The release job keeps a dormant `if: secrets.APPLE_CERTIFICATE != ''` branch so it can be enabled without restructuring. |
| No updater | **Copy for v1 (default D10)** | An auto-updater on a private repo needs authenticated downloads, and unsigned builds re-prompt for the keychain anyway. v1 shows an "update available" banner (GitHub releases API with the user's GitHub token) and the user installs the DMG by hand. `tauri-plugin-updater` + minisign comes later. |
| No fmt/clippy/test jobs | **Change: add them** | Agents write most of the code. `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, plus `bun run lint`, `tsc --noEmit`, and `vitest` are the cheapest guardrails. |
| Cargo.lock `--locked` check | **Copy** | Catches lockfile drift. |
| — | **Add** `cargo deny check` (licenses, advisories, bans) + Dependabot (cargo, npm, github-actions; weekly; grouped) | A desktop app that holds tokens needs supply-chain hygiene. |
| — | **Add** CSV parser test on placeholder fixtures | No company data in the repo (§7.5, §7.12). (No org-name grep guard: the AGENTS.md rule plus review cover it.) |
| `ubuntu-26.04` default runner | **Change: self-hosted Linux runners** | The repo lives in the harnesstap org, whose self-hosted runners cost no Actions minutes. Tauri on Linux needs `libwebkit2gtk-4.1-dev`, which the runner image must provide. |
| Playwright visual container, desktop-e2e | **Defer** | v1 tests the frontend with vitest + React Testing Library against mocked commands. Add tauri-driver e2e when the UI stabilises (Linux only; macOS has no WebDriver). |
| `rust-toolchain@stable` + `Swatinem/rust-cache@v2` + Bun 1.3.x | **Copy** | Pin the toolchain in `rust-toolchain.toml` (stable, with `rustfmt` and `clippy` components). |

### 11.3 Target workflows

**`.github/workflows/ci.yml`** (`permissions: contents: read`; `concurrency: ci-${{ github.ref }}` with cancel-in-progress for PRs)

| Job | Runner | Steps |
|---|---|---|
| `changes` | ubuntu-latest (GitHub-hosted) | `dorny/paths-filter`: outputs `rust` (`crates/**`, `app/src-tauri/**`, `Cargo.*`), `web` (`app/**` excl. src-tauri), `ci` (`.github/**`) |
| `rust` | self-hosted Linux | webkit deps check; `cargo fmt --all --check`; `cargo update --workspace --locked`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo nextest run --workspace` + doctests; `bun run tauri build --debug --no-bundle`. Dependencies from rust-cache; with `CI_CACHE_DIR` set, a persistent per-runner target dir (`.github/actions/persistent-cache`, capped by `CI_CACHE_MAX_GB`, default 14 GB; stale incremental/deps files go first) |
| `web` | self-hosted Linux | `bun install --frozen-lockfile`; `bun run lint`; `bun run typecheck`; `bun run test` |
| `desktop` | macos-latest, windows-latest (GitHub-hosted) | **Release tags `v*` and dispatch only.** macOS runs `cargo test` and builds the `.app` (7-day artifact); Windows runs `tauri build --no-bundle` |
| `deny` | ubuntu-latest (GitHub-hosted) | `cargo deny --all-features check` (prebuilt binary via `taiki-e/install-action`; no Docker needed) |
| `guards` | ubuntu-latest (GitHub-hosted) | actionlint (prebuilt binary; the runner runs in a container, so Docker actions cannot see the workspace); Changie fragment present unless the PR has the `no-changelog` label |
| `validate` | ubuntu-latest (GitHub-hosted), `if: always()` | fail if any needed job is `failure` or `cancelled` (skipped is fine); `desktop` is required on tags and dispatch |

**Runners.** Linux jobs run on the org's self-hosted runners: `runs-on: ${{ fromJSON(vars.LINUX_RUNNER_LABELS || '["self-hosted","Linux"]') }}`. The repository variable overrides the labels without a code change. The runners are shared, so `.github/actions/linux-tauri-deps` only **checks** the WebKitGTK headers there and fails with the `apt-get` command for a runner admin; it installs packages only on GitHub-hosted runners. Rust, Bun, Node 22 (`actions/setup-node`; the runner image ships an older Node), actionlint, and cargo-deny are installed per user by their setup actions, at pinned versions. `actions/checkout` cleans the workspace (`git clean -ffdx`) at the start of every job, so `target/` and `node_modules/` never leak between runs; Rust build caches come only from `Swatinem/rust-cache`.

**`nightly.yml`**: cron + dispatch. Linux jobs on the self-hosted runners; macOS and Windows builds on dispatch only. Runs the full CI with `cargo update` (no `--locked`) against the latest deps, plus `cargo deny`. It opens no issues. The status shows in the Actions tab.

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
     - **Asset name.** The product name is "PR Nexus", so Tauri builds `PR Nexus.app` and `PR Nexus_<ver>_aarch64.dmg`. GitHub would turn the space into a dot, so the reseal and upload steps give the DMG a stable, space-free name: `PR-Nexus_<ver>_aarch64.dmg`. The bundle identifier `dev.pr-nexus` (data folder, database, keychain item), the crate and binary names, env vars and the repository keep the `pr-nexus` form. `app/src/brand-name.test.ts` checks that the dashed name appears nowhere else.
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
| **M6** | Wizard + GitHub sync | `ProviderRegistry` (§5.1) with `setup_steps` unit tests. Setup wizard shell (§8.3): one step per screen, progress indicator, Back/Next, `onboarding.state` persistence and resume (including re-validation of completed steps). Steps 1–4 and 7 (a single preselected provider card each, GitHub then Slack; Connect GitHub with gh token, PAT, device flow; orgs picker with personal-repos toggle; Done with first sync). Tracked-orgs picker also in Settings; empty state until orgs are picked. Search buckets scoped by `org:` qualifiers, client-side tracked-org filter, status and nodes refresh, polling scheduler, rate-limit pause. HTTP fixture tests via `wiremock`. Typed errors mapped (401, 403-SSO, 403-secondary, 404). |
| **M7** | List UI | Dark compact style (§8.2). Sidebar navigation with the sync-status footer and refresh button. Grouped, collapsible list with three-level tri-state selection (global, section, row) and no per-row action buttons. No header overflow button. Inline project rename by double-click. Minimal section `⋯` menu (auto-link rules, archive, delete). Floating bulk bar. Slack request indicator with link icon. Filter menu (Grouping, Ordering, Show, Filters with Reset), persisted per view in `view_state`. Search bar over project name, title, body, repo, and number. Keyboard shortcuts. vitest: tri-state derivation (none/partial/all) for section and global; parent toggle selects or deselects only visible children; a filter or search excludes hidden rows from parent toggles; checking a multi-project PR updates every section containing it; bulk count and planner input dedupe by `PrId`; hidden-selected count; inline rename (Enter saves, Esc cancels, empty or duplicate name rejected, Unassigned not editable); filter and search predicates; view-state restore. |
| **M8** | Projects | CRUD, rules with live preview, manual/rule/excluded semantics tested. Unassigned section. Manual links by drag and drop and from the project dialog (D21). Relink after every sync and rule change. Header counts (§8.2). Empty project sections. Window state and release context menu (§8.2 Desktop shell). |
| **M9** | Ownership inputs | Files (paginated, capped), CODEOWNERS lookup order, `rules/branches`, protection with 403→Unknown, reviews with onBehalfOf, team members cache. Detail view shows readiness, owners, and the rationale. |
| **M10** | Slack single request/bump | **Spike first** (throwaway branch, findings written into §7.11): create an app from the **Create your Slack app** link and prove the code exchange with its client secret plus PKCE. Keep the PKCE-only check: on an app with PKCE enabled, prove the exchange without a secret. Then confirm that `127.0.0.1` loopback redirects are accepted, read `team.id`/`authed_user.id` from the token response (or via `slack_read_user_profile`), confirm refresh behaviour, and snapshot `tools/list`; record the Markdown read-back result (§7.6; titles are now code spans). Then wizard steps 5 and 6 (connect workspaces one at a time; mapping import or skip) and the same flows in Settings. Slack onboarding for **any number of workspaces** through the Slack MCP server: the **Create your Slack app** link in wizard step 5 and Settings, which opens `https://api.slack.com/apps?new_app=1&manifest_json=…` prefilled with `assets/slack-app-manifest.json` and the configured redirect URI, plus its help text; client ID, optional client secret (keychain), and redirect URI settings (global, optional per-workspace override, no client ID committed or shipped); the manifest validated in CI against Slack's vendored JSON schema plus the structure and link tests (§13); browser OAuth with a loopback listener on the configured port, `state` check, 10-minute timeout (2 minutes originally; raised after the live test), port-in-use error; tokens and refresh in the keychain vault; per-workspace re-auth; Remove never revokes. `SlackMcpChatProvider` on `rmcp` with the tool mapping of §7.11, the pinned-schema check on connect, and the typed error mapping. Workspace-scoped channel cache and picker (`slack_list_user_channels`, `slack_search_channels`); mapping rows routed to the right workspace session. Archived refusal from the archive status. `review_requests` lifecycle: created on request; bump updates; closed on merge/close, dismiss, supersede, or archived channel. Discovery via the MCP search tools with `include_bots=true` (background with throttle and cooldown, plus before the preview) stores `source='discovered_via_search'`. The PR detail's contextual Request review / Bump request button. Preview modal with Request/Bump cards → refresh-before-write → `slack_send_message`; permalink parsed from the send result. Bump in thread with `reply_broadcast`. Request indicator deep link (`slack://` with https permalink fallback). Corrections saved as user rows. Optional GitHub team review request. Tested against `FakeChat`, MCP contract fixtures, and OAuth tests (§13), plus one manual run in a test channel. No client ID, workspace, or company name in code, fixtures, or docs. **Delivered in M10:** browser OAuth through rmcp (discovery, PKCE S256, static client ID, optional keychain secret, refresh, loopback on the configured redirect URI, `xdg-open` on Linux), `auth.test` identity, the spike result in §7.11; wizard step 5 and Settings → Slack (Connect, Cancel, connected workspaces, Remove); the request/bump lifecycle with the byte-exact message builder, `slack_send_message`, stored thread `ts` and permalink, bump in thread, and the Requested/Bumped indicator with elapsed time and ↗ link; the preview dialog from the PR detail, the `r` key, and the bulk bar with one PR selected, with targets from active requests and mapping rows plus manual channel ID entry. **Deferred to a later milestone:** the channel cache and picker, discovery via search, batch request/bump across several PRs, archived-channel refusal, per-workspace client override, correction rows, optional GitHub team review request, the `slack://` deep link (the https permalink is used), and the manual run in a test channel. |
| **M11** | Bulk | Single bulk action **Request / bump on Slack**. Per-channel grouping with request and bump sections per card, bulk bump grouping (§7.9), result report, retry failed. Planner unit tests for mixed cases (requested + never requested in one channel, 1/≤3/>3 roots, mixed workspaces, a closed request → new request). **Delivered in M11:** the bottom-bar button now works for any selection (one PR keeps the single preview; two or more open the bulk preview); the core planner `review::batch` (gate → owners → mapped channels → group by `(workspace, channel)`, bump per root up to `review.bulk_bump_max_threads` = 3, one remind above that with the old rows `superseded`, one request for never-requested PRs, per-card "One top-level request for all" switch, recent-bump warning); a preview with every message's exact text per channel card plus the skipped PRs and why; an optional "All to one channel" target that accepts a typed channel or DM ID (`C…`, `G…`, `D…`); refresh-before-write per card (a card whose plan changed since the preview is skipped with "Changed since the preview"); sequential sends spaced 1.1 s apart per channel, one wait-and-retry on `RateLimited` when Slack asks for 60 s or less, `send_attempts` under one `batch_id` (`bulk-<time>`), no rollback; the result table (PR, channel, outcome, detail, permalink) with **Retry failed**, which runs the gate again for the failed PRs only. **Deferred:** the toast after the dialog closes (the list indicators update in place), the optional GitHub team review request in bulk, and channel picking per PR inside the bulk preview (PRs without a mapped channel are skipped with a hint to request them alone or send all to one channel). |
| **M12** | Context panel | Threads for known messages, cache, user names, Mentions opt-in. **Delivered in M12:** the "Slack" panel beside the code owners in the PR detail (the dialog widens); roots from `review_requests` (active, then closed with the close reason) and `review_messages` (posted), newest first, deduplicated per `(channel, root)`; `slack_read_thread` with `limit` 50 and a "first 50 replies" note when Slack reports more; `chat_thread_cache` for 5 min with **Refresh** to bypass it; author names from `slack_read_user_profile`, cached per session and stored in the cached thread; per-message permalinks; Slack mrkdwn shown as plain text (never HTML); per-thread errors (signed out → "Reconnect in Settings → Slack", rate limited → retry time) never fail the panel, and the stale cached copy stays visible; a rejected token marks the workspace as needing sign-in. Settings → Slack → **Find mentions**: Off (default), Public channels (`slack_search_public`), or Public, private channels and DMs (`slack_search_public_and_private`); one search per signed-in workspace per click, phrase `"github.com/ORG/REPO/pull/N"`, `include_bots=true`, 20 results, newest first, messages PR Nexus already knows for the PR dropped. The read and search tools are not in the pinned-schema check (a change there only breaks the panel, never a send); emails in author fields are never kept. Demo mode returns a fake thread. |
| **M13** | Polish | Settings complete, log redaction test, "update available" banner (GitHub releases API with the user's token, manual DMG download), keychain re-prompt explainer, Settings → About → Re-run setup, the skipped-mapping hint, empty states. **Delivered in M13:** Settings as tabs (Accounts, Mappings, Review, Sync, About; arrow keys, Home and End move between tabs). Accounts holds the providers, the tracked orgs and Settings → Slack. Mappings lists every team with an Imported/Edited badge and a filter, edits a row inline (channel name or ID, workspace, Disabled) as a user row, adds a team, resets an edited row to the imported one, imports a CSV (merge or replace) with a dry-run diff (new, changed field by field, unchanged, and imported rows your edits still shadow) where **Apply** stays disabled until the same CSV, workspace and mode were previewed, and exports the effective, user or imported rows. Review and Sync edit the §7.12 keys with range checks in the backend (`save_preferences` validates every key before writing any; an empty ticket pattern restores the default); **Also post bumps to the channel** moved from Slack to Review. About shows the version, repository, database and log paths with Copy, the keychain explainer (one item; unsigned builds make macOS ask once after each update, choose Always Allow; a warning when tokens are in memory only), the unsigned-install note, **Check for updates** and **Re-run setup** (the wizard from step 1 with **Back to PR Nexus**; setup stays marked finished during a re-run). Logs: a daily file in the OS log folder, 7 kept, every line through `redact` (Authorization headers, OAuth `code`, `code_verifier`, `client_secret`, access and refresh tokens in JSON or query strings, `ghp_`/`github_pat_` and `xox?-` tokens become `[REDACTED]`), with unit tests. Update banner: on launch, `GET /repos/OWNER/REPO/releases/latest` with the user's GitHub token (never in demo mode, silent on failure), shown when the release is newer and not a draft or pre-release; it links the release page and copies `gh release download vX.Y.Z -R OWNER/REPO -p '*.dmg'`; dismissing hides that version only. The skipped-mapping hint shows above the list while no team is mapped and a workspace is connected, opens Settings → Mappings, and can be dismissed. The list shows loading and load-error states with **Try again**; an empty list says whether the view has no open PRs or the search and filters hide them, with **Clear search** and **Reset filters**. List polish against `docs/ui/list-view.jpg`: `N open PRs` beside the global checkbox, violet checkboxes, a focused-row accent, and the selection bar as a wide bar with `N selected`, **Request / bump on Slack** with the Slack mark, and **✕ Clear**. Keyboard and labels: the shortcuts above, `aria-current` on the focused row, row, list and selection-bar landmarks, a polite live count in the selection bar, and `aria-expanded` on the sync status. |

M0–M1 come first on purpose. Every later PR is then built, linted, and releasable from day one, which matters when coding agents produce most of the commits.

## 13. Testing strategy

- **core**: pure unit tests and golden files (messages, CODEOWNERS, decisions, mapping CSV). `insta` snapshots for `OwnerDecision` JSON. Property tests (`proptest`) for the CODEOWNERS matcher against a small reference implementation of gitignore-style matching.
- **providers**: `wiremock` HTTP fixtures, recorded once from real responses and scrubbed of names, IDs, and tokens. Each GitHub fixture covers one shape: search page, PR fragment, 403 SSO, secondary rate limit.
- **Slack MCP contract tests**: recorded `initialize`, `tools/list`, and `tools/call` request/response pairs, replayed by `wiremock` on a fake Streamable HTTP endpoint. One fixture per shape:
  - send (top level, and in a thread with `reply_broadcast`), with permalink parsing;
  - the official result format (`slack-mcp-send-message.json`: JSON text with `message_context`; `slack-mcp-read-channel.json`: message blocks with escaped links and the "Sent using" attribution), an unreadable send result recovered from `slack_read_channel`, and one that ends as "sent, link unknown";
  - search hit, no hit, and a bot-posted hit;
  - `slack_search_channels` with an archived channel, and a `slack_list_user_channels` page;
  - `slack_read_thread`, `slack_read_user_profile`;
  - `isError` with `ratelimited`, `not_in_channel`, `missing_scope`;
  - 401 challenge, then refresh, then retry.
  Assertions check the exact tool arguments sent (`include_bots=true`, `reply_broadcast`, `thread_ts`, standard-Markdown `message`) and that the pinned schema rejects a `tools/list` with a renamed tool or a new required field. A scrubbing test fails if a fixture contains a real-looking client ID or team ID.
- **Slack OAuth tests**:
  - the discovery documents;
  - the authorize URL (PKCE S256, `state`, scopes, `redirect_uri` exactly as configured);
  - a callback with the wrong `state` is rejected;
  - token exchange with and without a client secret, and refresh; the secret is sent as `client_secret_post` only when configured and never appears in logs;
  - port-in-use error;
  - tokens land only in the (mock) keyring.
- **Slack app manifest** (runs in `cargo test`, so in CI, offline):
  - `assets/slack-app-manifest.json` validates against Slack's published JSON schema, vendored at `assets/schemas/slack-manifest.schema.1.0.0.json` from `slackapi/manifest-schema` (MIT), pinned to a tag (v0.6.0 includes `settings.is_mcp_enabled`, `oauth_config.pkce_enabled`, and `scopes.user_optional`), with the `jsonschema` crate (0.60);
  - a structure test checks that the name is `PR Nexus`; that `scopes.user` equals the scope list of §7.11 and `user_optional ⊆ user`; that there are no bot scopes and no bot user; that `is_mcp_enabled` is `true` and `pkce_enabled` is `false`; that `redirect_urls[0]` equals the `slack.mcp.redirect_uri` default; and that it contains no client ID, team ID, or company name;
  - a link test decodes the built URL, checks the `https://api.slack.com/apps?new_app=1&manifest_json=` prefix, round-trips the JSON, and checks the configured redirect URI replaced the default;
  - `apps.manifest.validate` needs an app configuration token, so it stays a manual pre-release step, not CI.
- **wizard**: core tests for `setup_steps` (order, only `ConnectChat` and `ChatMapping` skippable, kinds without a descriptor contribute no steps, v1 registers only GitHub and Slack) and for the resume logic (state round-trip, resume at the stored step, fall back to the first step whose re-validation fails). vitest: one step rendered at a time, Next disabled until the step's condition holds, Back disabled on step 1, Skip only on the Slack and mapping steps, steps 1 and 2 show only the GitHub and Slack card, preselected, Slack sign-in (custom redirect saved before connect, waiting state with Cancel, connected workspace name, Next enabled, cancel shows no error, errors inline, Remove), and adding a second Slack workspace.
- **review requests**: table-driven lifecycle tests in core: sync sees merged/closed → closed; dismiss → next action Request; new commits after approval → still Bump; reopened PR → Request; discovery hit on a closed root → ignored; `/pull/4` vs `/pull/42`; oldest root wins; thread replies count as bumps.
- **app**: command-level tests with `FakeVcs`/`FakeChat` and a temp DB. Frontend tests with vitest + Testing Library against a mocked `invoke`.
- **Manual pre-release checklist**: open **Create your Slack app**, confirm Slack shows the prefilled manifest (name, user scopes, redirect URL, MCP on), and run `apps.manifest.validate` on it; sign in to a test workspace through the Slack MCP server; request and bump in a private test channel; verify link rendering, Markdown-active characters in a title, thread plus channel broadcast, permalink, and that the row's ↗ opens the original message in the Slack app; run the pinned-schema check against the live server.
- There is no live-API test in CI.

## 14. Open questions: answered (A) and defaults chosen (D)

**Answered by the owner (2026-10-25)**
- **A1. No company names or data in the repo or spec.** GitHub orgs and Slack workspaces are chosen by the user in onboarding and Settings; nothing is hardcoded or defaulted. The team → channel mapping starts empty (no committed seed). Names stay out of code (§7.12).
- **A2. Multi-workspace Slack in v1 (N workspaces):** per-workspace OAuth grant and tokens in the keyring vault, `slack_workspace` on mapping rows, workspace-scoped channel cache and picker, `(workspace, channel)` grouping (§7.11).
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
- **D8. The Slack MCP search tools discover requests posted outside the app** (`slack_search_public`, then `slack_search_public_and_private`, with `include_bots=true`; §7.4). Without the search scopes, only app-posted requests are known.
- **D9. CSV workspace:** the CSV may carry a `slack_workspace` column. Rows without one get the target workspace the user picks in the import dialog.
- **D10. No auto-updater in v1:** an "update available" banner, and manual DMG install.
- **D11. Linux/Windows build in CI only,** with no release assets.
- **D12. Bump interval:** a warning (not a block) below 24 h.
- **D13. No Slack drafts:** the in-app preview plus an explicit Send replaces the skill's draft-first flow.
- **D14. Review-request lifecycle:** a request is closed only by merge/close, explicit dismiss, being superseded by a new top-level message in the same channel, or an archived channel. Approvals and new commits do not close it, and there is no age expiry (§7.4).
- **D15. Discovery cadence:** background search at most once per PR per workspace every 24 h, ≤10 calls/min per workspace, plus a forced check before the preview (§7.4).
- **D16. List actions:** no per-row action buttons and no header overflow button; the bulk bar is the only list action, and the PR detail keeps a contextual button. Projects are renamed by double-clicking the title (§7.7, §8.2).
- **D21. Manual project links:** drag rows (or the selection) onto a sidebar project or a section header, or use the project dialog's Add pull request search. No per-row buttons and no extra bulk action (§8.1).
- **D22. Desktop shell:** first launch maximized, then the last window geometry is restored; no native context menu in release builds (§8.2).
- **D17. Selection:** three-level tri-state checkboxes over visible rows, keyed by `PrId` across sections, session-only (§8.2).
- **D18. Setup wizard:** separate screens, one step at a time, driven by the provider registry; GitHub and Slack are the only providers and the only ones shown (steps 1 and 2 keep a single preselected card so step numbering stays stable); no future provider appears in the UI; only the Slack connect and mapping steps are skippable; progress is persisted and resumable (§5.1, §8.3).
- **D19. Slack transport = the official Slack MCP server** (`https://mcp.slack.com/mcp`). PR Nexus is an `rmcp` client with OAuth 2.1 + PKCE and a loopback redirect. The client ID, client secret (keychain), and redirect URI are user configuration (global, or per workspace). Direct Web API with a per-workspace internal app is the documented fallback, not built in v1 (§7.11, §7.13).
- **D20. No default Slack client ID ships** in the app, the repo, fixtures, or docs. The client ID field starts empty. Users create their own app with the **Create your Slack app** link (a manifest committed at `assets/slack-app-manifest.json`), or paste the client ID of an MCP-enabled app their org already approved (§7.11).
