/** Pending reviewer teams are `org/slug`. Older filters may store only the slug or the team name. */

export interface TeamOption {
  slug: string;
  name: string;
  count: number;
}

export interface TeamChoiceLabel {
  id: string;
  label: string;
  /** Org, when the name or slug is shared with another team. */
  hint?: string;
  /** Shown after the hint, so the count stays at the end of the row. */
  countLabel?: string;
}

const LOGIN = /^[A-Za-z0-9._-]+$/u;

export function parseTeamId(value: string): { org: string; slug: string } | { org: ""; slug: string } | null {
  const trimmed = value.trim().replace(/^@/, "");
  if (trimmed === "") {
    return null;
  }
  const parts = trimmed.split("/");
  if (parts.length === 2 && parts[0] && parts[1] && LOGIN.test(parts[0]) && LOGIN.test(parts[1])) {
    return { org: parts[0], slug: parts[1] };
  }
  if (parts.length === 1) {
    return { org: "", slug: trimmed };
  }
  return null;
}

export function teamSlug(value: string): string {
  return (parseTeamId(value)?.slug ?? value.trim()).toLowerCase();
}

/** A pending team login matches a filter value. A bare slug matches every org with that slug. */
export function teamMatches(haveLogin: string, filter: string, haveName?: string | null): boolean {
  const want = parseTeamId(filter);
  if (!want) {
    return false;
  }
  if (haveLogin.toLowerCase() === filter.trim().toLowerCase()) {
    return true;
  }
  const have = parseTeamId(haveLogin);
  if (want.org !== "") {
    return have !== null && have.org.toLowerCase() === want.org.toLowerCase() && have.slug.toLowerCase() === want.slug.toLowerCase();
  }
  if (have && have.slug.toLowerCase() === want.slug.toLowerCase()) {
    return true;
  }
  return (haveName ?? "").trim().toLowerCase() === filter.trim().toLowerCase() && filter.trim() !== "";
}

/**
 * Turns saved filter values into `org/slug` logins from `known`.
 * An unambiguous bare slug or name becomes that login. A shared one becomes
 * every match, so a saved "platform" filter still covers both orgs.
 */
export function expandTeamFilters(values: readonly string[], known: readonly { login: string; name?: string | null }[]): string[] {
  const out: string[] = [];
  for (const value of values) {
    const matches = known.filter((team) => teamMatches(team.login, value, team.name));
    if (matches.length === 0) {
      pushUnique(out, value.trim());
    } else {
      for (const team of matches) {
        pushUnique(out, team.login);
      }
    }
  }
  return out;
}

function pushUnique(out: string[], value: string): void {
  if (value === "" || out.some((have) => have.toLowerCase() === value.toLowerCase())) {
    return;
  }
  out.push(value);
}

/** Labels for the reviewer-team menu. Unambiguous teams keep today's "Name · org/slug (n)". Shared names or slugs show the name and a muted org. */
export function teamFilterLabels(teams: readonly TeamOption[]): TeamChoiceLabel[] {
  const bySlug = countBy(teams, (team) => teamSlug(team.slug));
  const byName = countBy(teams, (team) => (team.name || teamSlug(team.slug)).toLowerCase());
  return teams.map((team) => {
    const short = teamSlug(team.slug);
    const org = parseTeamId(team.slug)?.org ?? "";
    const title = team.name || short;
    const ambiguous = (bySlug.get(short) ?? 0) > 1 || (byName.get(title.toLowerCase()) ?? 0) > 1;
    if (ambiguous) {
      return { id: team.slug, label: title, hint: org || team.slug, countLabel: `(${team.count})` };
    }
    return { id: team.slug, label: team.name ? `${team.name} · ${team.slug} (${team.count})` : `${team.slug} (${team.count})` };
  });
}

function countBy(teams: readonly TeamOption[], key: (team: TeamOption) => string): Map<string, number> {
  const counts = new Map<string, number>();
  for (const team of teams) {
    const id = key(team);
    counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  return counts;
}

export function teamFilterSelected(selected: readonly string[], id: string): boolean {
  return selected.some((value) => teamMatches(id, value) || teamMatches(value, id));
}

/** Expand any bare values against known teams, then toggle `id`. */
export function toggleTeamFilter(selected: readonly string[], id: string, known: readonly TeamOption[]): string[] {
  const expanded = expandTeamFilters(
    selected,
    known.map((team) => ({ login: team.slug, name: team.name })),
  );
  return expanded.includes(id) ? expanded.filter((item) => item !== id) : [...expanded, id];
}
