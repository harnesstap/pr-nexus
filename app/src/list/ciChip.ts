import type { FailingCheck, ListRow } from "../api";

/** Check names longer than this are cut, ellipsis included, so the chip stays compact. */
export const CHECK_NAME_LIMIT = 18;

type Conclusion = FailingCheck["conclusion"];

export interface CiFailureChip {
  /** Chip text, for example "lint failed" or "3 checks failed". */
  label: string;
  /**
   * Truncated check name when exactly one failed. The badge keeps {@link mark}
   * visible and lets this part take the ellipsis.
   */
  name: string | null;
  /** The verb for one check, for example "failed" or "timed out". */
  mark: string | null;
  /** One line per failing check: name and conclusion. */
  lines: string[];
  /** What a click does, including which check opens when there are several. */
  detail: string;
  /** Accessible reason after the label. */
  summary: string;
  /** Details URL of the check a click opens, or the pull request's checks page. */
  url: string;
}

/** The visible name, cut with an ellipsis past {@link CHECK_NAME_LIMIT}. */
export function truncateCheckName(name: string): string {
  const chars = [...name.trim()];
  if (chars.length <= CHECK_NAME_LIMIT) {
    return chars.join("");
  }
  return `${chars.slice(0, CHECK_NAME_LIMIT - 1).join("")}…`;
}

function knownConclusion(value: string): Conclusion | null {
  switch (value) {
    case "failure":
    case "timed_out":
    case "cancelled":
    case "action_required":
      return value;
    default:
      return null;
  }
}

function verb(conclusion: Conclusion): string {
  switch (conclusion) {
    case "failure":
      return "failed";
    case "timed_out":
      return "timed out";
    case "cancelled":
      return "cancelled";
    case "action_required":
      return "needs action";
    default: {
      const unreachable: never = conclusion;
      return unreachable;
    }
  }
}

function conclusionWord(conclusion: Conclusion): string {
  switch (conclusion) {
    case "failure":
      return "failure";
    case "timed_out":
      return "timed out";
    case "cancelled":
      return "cancelled";
    case "action_required":
      return "action required";
    default: {
      const unreachable: never = conclusion;
      return unreachable;
    }
  }
}

/**
 * Label, tooltip, and click target for a failing rollup. `null` when no check
 * name is cached yet, so the chip stays "Fix CI" and opens the checks page.
 */
export function ciFailureChip(row: Pick<ListRow, "url" | "failing_checks">): CiFailureChip | null {
  const raw = row.failing_checks;
  if (!Array.isArray(raw)) {
    return null;
  }
  const checks = raw.flatMap((check) => {
    if (!check || typeof check.name !== "string") {
      return [];
    }
    const name = check.name.trim();
    if (name.length === 0) {
      return [];
    }
    const url = typeof check.url === "string" ? check.url.trim() : "";
    return [{ name, conclusion: knownConclusion(check.conclusion) ?? "failure", url: url || null }];
  });
  if (checks.length === 0) {
    return null;
  }
  const lines = checks.map((check) => `${check.name} · ${conclusionWord(check.conclusion)}`);
  const first = checks[0];
  const url = first.url ?? `${row.url}/checks`;
  const detail = first.url
    ? checks.length === 1
      ? `Click to open the ${first.name} log`
      : `Click to open the ${first.name} log, the first failing check`
    : "Click to open the checks page";
  const single = checks.length === 1;
  const name = single ? truncateCheckName(first.name) : null;
  const mark = single ? verb(first.conclusion) : null;
  const label = single ? `${name} ${mark}` : `${checks.length} checks failed`;
  return { label, name, mark, lines, detail, summary: `${lines.join("; ")}. ${detail}`, url };
}
