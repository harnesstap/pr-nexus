/** Release notes embedded at build time from Changie's `.changes/` files (§8.2 Changelog). */

export interface ChangelogSection {
  kind: string;
  entries: string[];
}

export interface ChangelogRelease {
  /** Without the leading `v`, e.g. `0.1.1`. `null` for unreleased fragments. */
  version: string | null;
  /** `YYYY-MM-DD`, or `null` when unknown. */
  date: string | null;
  sections: ChangelogSection[];
}

/** Section order follows `.changie.yaml`; unknown kinds go last, alphabetically. */
const KIND_ORDER = ["Breaking", "Features", "Added", "Changed", "Fixes", "Fixed", "Docs", "Dependencies"];

function kindRank(kind: string): number {
  const index = KIND_ORDER.indexOf(kind);
  return index === -1 ? KIND_ORDER.length : index;
}

function sortSections(sections: ChangelogSection[]): ChangelogSection[] {
  return [...sections].sort((a, b) => kindRank(a.kind) - kindRank(b.kind) || a.kind.localeCompare(b.kind));
}

/** Parses one `.changes/vX.Y.Z.md` file (or a whole CHANGELOG.md) into releases. */
export function parseChangelog(markdown: string): ChangelogRelease[] {
  const releases: ChangelogRelease[] = [];
  let release: ChangelogRelease | null = null;
  let section: ChangelogSection | null = null;
  let entry = false;
  for (const raw of markdown.split(/\r?\n/)) {
    const line = raw.trimEnd();
    const heading = line.match(/^##\s+v?(\d+\.\d+\.\d+[^\s]*)(?:\s+-\s+(\d{4}-\d{2}-\d{2}))?\s*$/);
    if (heading) {
      release = { version: heading[1], date: heading[2] ?? null, sections: [] };
      releases.push(release);
      section = null;
      entry = false;
      continue;
    }
    if (!release) {
      continue;
    }
    const kind = line.match(/^###\s+(.+)$/);
    if (kind) {
      section = { kind: kind[1].trim(), entries: [] };
      release.sections.push(section);
      entry = false;
      continue;
    }
    if (!section) {
      continue;
    }
    const item = line.match(/^[-*]\s+(.*)$/);
    if (item) {
      section.entries.push(item[1].trim());
      entry = true;
    } else if (entry && /^\s+\S/.test(line)) {
      section.entries[section.entries.length - 1] += ` ${line.trim()}`;
    } else if (!line.trim()) {
      entry = false;
    }
  }
  return releases.map((item) => ({ ...item, sections: sortSections(item.sections.filter((s) => s.entries.length > 0)) }));
}

/** Reads `kind` and `body` from one Changie fragment (`.changes/unreleased/*.yaml`). */
export function parseFragment(yaml: string): { kind: string; body: string } | null {
  const lines = yaml.split(/\r?\n/);
  let kind: string | null = null;
  let body: string | null = null;
  for (let index = 0; index < lines.length; index++) {
    const match = lines[index].match(/^(kind|body):\s*(.*)$/);
    if (!match) {
      continue;
    }
    let value = match[2].trim();
    if (/^[|>][-+]?$/.test(value)) {
      const block: string[] = [];
      while (index + 1 < lines.length && (/^\s+/.test(lines[index + 1]) || !lines[index + 1].trim())) {
        block.push(lines[++index].trim());
      }
      value = block.filter(Boolean).join(" ");
    } else if (/^(["']).*\1$/.test(value)) {
      value = value.slice(1, -1);
    }
    if (match[1] === "kind") {
      kind = value;
    } else {
      body = value;
    }
  }
  return kind && body ? { kind, body } : null;
}

/** Groups unreleased fragments into one `Unreleased` entry, or null when there are none. */
export function unreleasedRelease(fragments: string[]): ChangelogRelease | null {
  const byKind = new Map<string, string[]>();
  for (const text of fragments) {
    const fragment = parseFragment(text);
    if (fragment) {
      byKind.set(fragment.kind, [...(byKind.get(fragment.kind) ?? []), fragment.body]);
    }
  }
  if (byKind.size === 0) {
    return null;
  }
  return { version: null, date: null, sections: sortSections([...byKind].map(([kind, entries]) => ({ kind, entries }))) };
}

function parts(version: string): { numbers: number[]; pre: string | null } | null {
  const match = version.trim().replace(/^v/, "").match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$/);
  if (!match) {
    return null;
  }
  return { numbers: [Number(match[1]), Number(match[2]), Number(match[3])], pre: match[4] ?? null };
}

/** Semver order (`v` prefix allowed). A pre-release sorts before its release. Unparseable versions compare equal. */
export function compareVersions(a: string, b: string): number {
  const left = parts(a);
  const right = parts(b);
  if (!left || !right) {
    return 0;
  }
  for (let index = 0; index < 3; index++) {
    if (left.numbers[index] !== right.numbers[index]) {
      return left.numbers[index] - right.numbers[index];
    }
  }
  if (left.pre === right.pre) {
    return 0;
  }
  if (left.pre === null) {
    return 1;
  }
  if (right.pre === null) {
    return -1;
  }
  return left.pre.localeCompare(right.pre, undefined, { numeric: true });
}

/** Released versions, newest first. */
export function sortReleases(releases: ChangelogRelease[]): ChangelogRelease[] {
  return [...releases].filter((item) => item.version).sort((a, b) => compareVersions(b.version!, a.version!));
}

export type WhatsNewDecision =
  /** Nothing to show; store `record` as the last seen version when it is set. */
  | { show: false; record: string | null }
  | { show: true; releases: ChangelogRelease[] };

/**
 * Whether to open "What's new" on launch. A first launch (nothing stored) records the
 * version silently. An upgrade shows every release in (lastSeen, current]. A downgrade
 * or an upgrade with no embedded notes just records the version, so a dev rebuild of
 * the same version never shows anything.
 */
export function decideWhatsNew(current: string, lastSeen: string | null, releases: ChangelogRelease[]): WhatsNewDecision {
  if (!parts(current)) {
    return { show: false, record: null };
  }
  if (lastSeen === null || !parts(lastSeen)) {
    return { show: false, record: current };
  }
  const order = compareVersions(current, lastSeen);
  if (order === 0) {
    return { show: false, record: null };
  }
  if (order < 0) {
    return { show: false, record: current };
  }
  const pending = sortReleases(releases).filter(
    (item) => compareVersions(item.version!, lastSeen) > 0 && compareVersions(item.version!, current) <= 0 && item.sections.length > 0,
  );
  return pending.length > 0 ? { show: true, releases: pending } : { show: false, record: current };
}

const LAST_SEEN_KEY = "pr-nexus.last-seen-version";

/** Per-machine UI state, like the dismissed banners in `list/hints.ts`. */
export function lastSeenVersion(): string | null {
  try {
    return window.localStorage.getItem(LAST_SEEN_KEY);
  } catch {
    return null;
  }
}

export function recordSeenVersion(version: string) {
  try {
    window.localStorage.setItem(LAST_SEEN_KEY, version.replace(/^v/, ""));
  } catch {
    // Private mode or a full disk: the modal may show again next launch.
  }
}
