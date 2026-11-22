import { expect, test } from "vitest";
import type { FailingCheck } from "../api";
import { CHECK_NAME_LIMIT, ciFailureChip, truncateCheckName } from "./ciChip";

function check(partial: Partial<FailingCheck> & Pick<FailingCheck, "name" | "conclusion">): FailingCheck {
  return { url: `https://github.com/acme/widgets/actions/runs/${partial.name}`, ...partial };
}

const row = { url: "https://github.com/acme/widgets/pull/7" };

test("one failing check is named, and a click opens its log", () => {
  const chip = ciFailureChip({ ...row, failing_checks: [check({ name: "lint", conclusion: "failure" })] });
  expect(chip).toMatchObject({
    label: "lint failed",
    name: "lint",
    mark: "failed",
    lines: ["lint · failure"],
    detail: "Click to open the lint log",
    url: "https://github.com/acme/widgets/actions/runs/lint",
  });
  expect(chip?.summary).toBe("lint · failure. Click to open the lint log");
});

test("timed out, cancelled, and action required use their own words", () => {
  expect(ciFailureChip({ ...row, failing_checks: [check({ name: "integration", conclusion: "timed_out" })] })?.label).toBe("integration timed out");
  expect(ciFailureChip({ ...row, failing_checks: [check({ name: "nightly", conclusion: "cancelled" })] })?.lines).toEqual(["nightly · cancelled"]);
  const action = ciFailureChip({ ...row, failing_checks: [check({ name: "deploy", conclusion: "action_required" })] });
  expect(action?.label).toBe("deploy needs action");
  expect(action?.lines).toEqual(["deploy · action required"]);
});

test("several failing checks count them and open the first log", () => {
  const chip = ciFailureChip({
    ...row,
    failing_checks: [
      check({ name: "lint", conclusion: "failure" }),
      check({ name: "typecheck", conclusion: "timed_out" }),
      { name: "deploy", conclusion: "cancelled", url: null },
    ],
  });
  expect(chip?.label).toBe("3 checks failed");
  expect(chip?.name).toBeNull();
  expect(chip?.mark).toBeNull();
  expect(chip?.lines).toEqual(["lint · failure", "typecheck · timed out", "deploy · cancelled"]);
  expect(chip?.detail).toBe("Click to open the lint log, the first failing check");
  expect(chip?.url).toBe("https://github.com/acme/widgets/actions/runs/lint");
});

test("a long check name is cut with an ellipsis", () => {
  const name = "release-please-and-publish-notes";
  expect(truncateCheckName(name).endsWith("…")).toBe(true);
  expect([...truncateCheckName(name)].length).toBe(CHECK_NAME_LIMIT);
  const chip = ciFailureChip({ ...row, failing_checks: [check({ name, conclusion: "failure" })] });
  expect(chip?.label).toBe(`${truncateCheckName(name)} failed`);
  expect(chip?.name).toBe(truncateCheckName(name));
  expect(chip?.mark).toBe("failed");
  // The tooltip keeps the full name.
  expect(chip?.lines[0]).toBe(`${name} · failure`);
});

test("a check with no log opens the checks page, and says so", () => {
  const chip = ciFailureChip({ ...row, failing_checks: [{ name: "lint", conclusion: "failure", url: "  " }] });
  expect(chip?.url).toBe("https://github.com/acme/widgets/pull/7/checks");
  expect(chip?.detail).toBe("Click to open the checks page");
});

test("missing or empty names keep the generic chip", () => {
  expect(ciFailureChip(row)).toBeNull();
  expect(ciFailureChip({ ...row, failing_checks: null })).toBeNull();
  expect(ciFailureChip({ ...row, failing_checks: [] })).toBeNull();
  expect(ciFailureChip({ ...row, failing_checks: [{ name: "  ", conclusion: "failure" }] })).toBeNull();
});

test("an unknown conclusion still names the check as a failure", () => {
  const chip = ciFailureChip({
    ...row,
    failing_checks: [{ name: "lint", conclusion: "startup_failure" as FailingCheck["conclusion"] }],
  });
  expect(chip?.label).toBe("lint failed");
  expect(chip?.lines).toEqual(["lint · failure"]);
});
