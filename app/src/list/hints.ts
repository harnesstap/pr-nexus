/** Dismissed banners live in localStorage: they are per-machine UI state, not settings. */
const UPDATE_KEY = "pr-nexus.update-dismissed";
const MAPPING_KEY = "pr-nexus.mapping-hint-dismissed";

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Private mode or a full disk: the banner just comes back next launch.
  }
}

/** Dismissing hides one version only; the next release shows the banner again. */
export function updateDismissed(version: string): boolean {
  return read(UPDATE_KEY) === version;
}

export function dismissUpdate(version: string) {
  write(UPDATE_KEY, version);
}

export function mappingHintDismissed(): boolean {
  return read(MAPPING_KEY) === "1";
}

export function dismissMappingHint() {
  write(MAPPING_KEY, "1");
}

/** §8.2: shown when no team is mapped yet but Slack is connected. */
export function showMappingHint(mappingCount: number, slackWorkspaces: number): boolean {
  return mappingCount === 0 && slackWorkspaces > 0 && !mappingHintDismissed();
}
