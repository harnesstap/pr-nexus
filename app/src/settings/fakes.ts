import type { Api, Preferences } from "../api";

/** The §7.12 defaults, as the backend returns them on a fresh database. */
export const DEFAULT_PREFERENCES: Preferences = {
  review_requester: "",
  min_bump_interval_hours: 24,
  bulk_bump_max_threads: 3,
  bump_broadcast: true,
  blocked_review_required_is_ready: true,
  ticket_key_regex: "([A-Z][A-Z0-9]+-\\d+)",
  focused_interval_s: 180,
  unfocused_interval_s: 900,
  keep_closed_days: 7,
  live_updates: true,
};

type SettingsApi = Pick<
  Api,
  | "loadPreferences"
  | "savePreferences"
  | "listMappings"
  | "saveMapping"
  | "resetMapping"
  | "previewMappingImport"
  | "applyMappingImport"
  | "exportMapping"
  | "appInfo"
  | "checkUpdate"
  | "slackWorkspaces"
  | "pickMappingFile"
>;

/** In-memory Settings endpoints for tests and the browser preview. */
export function fakeSettingsApi(overrides: Partial<SettingsApi> = {}): SettingsApi {
  return {
    loadPreferences: () => Promise.resolve(DEFAULT_PREFERENCES),
    savePreferences: (prefs) => Promise.resolve(prefs),
    listMappings: () => Promise.resolve([]),
    saveMapping: () => Promise.resolve(),
    resetMapping: () => Promise.resolve(true),
    previewMappingImport: () => Promise.resolve({ added: [], changed: [], unchanged: 0, user_shadowed: [] }),
    applyMappingImport: () => Promise.resolve({ rows: 0 }),
    exportMapping: () => Promise.resolve(""),
    appInfo: () =>
      Promise.resolve({
        version: "0.1.0",
        repository: "acme/pr-nexus",
        db_path: "/Users/ada/Library/Application Support/pr-nexus/pr-nexus.db",
        log_dir: "/Users/ada/Library/Logs/pr-nexus",
        keyring_in_memory: false,
        os: "macos",
      }),
    checkUpdate: () => Promise.resolve(null),
    slackWorkspaces: () => Promise.resolve([]),
    pickMappingFile: () => Promise.resolve(null),
    ...overrides,
  };
}
