import { beforeEach, describe, expect, it } from "vitest";
import { compareVersions, decideWhatsNew, lastSeenVersion, parseChangelog, parseFragment, recordSeenVersion, unreleasedRelease, type ChangelogRelease } from "./changelog";
import { RELEASES } from "./data";

const md = `## v0.2.0 - 2026-12-01

### Fixed

- A fix.

### Added

- First feature
  wrapped on two lines.
- Second feature.

### Empty
`;

function release(version: string): ChangelogRelease {
  return { version, date: null, sections: [{ kind: "Added", entries: [`in ${version}`] }] };
}

describe("parseChangelog", () => {
  it("reads version, date and sections in Changie kind order", () => {
    expect(parseChangelog(md)).toEqual([
      {
        version: "0.2.0",
        date: "2026-12-01",
        sections: [
          { kind: "Added", entries: ["First feature wrapped on two lines.", "Second feature."] },
          { kind: "Fixed", entries: ["A fix."] },
        ],
      },
    ]);
  });

  it("reads several versions from a CHANGELOG.md, skipping the header", () => {
    const parsed = parseChangelog(`# Changelog\n\nIntro text.\n\n${md}\n## v0.1.0 - 2026-10-01\n\n### Changed\n\n- Old.\n`);
    expect(parsed.map((item) => item.version)).toEqual(["0.2.0", "0.1.0"]);
  });

  it("embeds the repo's .changes files at build time", () => {
    expect(RELEASES.length).toBeGreaterThan(0);
    expect(RELEASES.map((item) => item.version)).toContain("0.1.0");
    for (let index = 1; index < RELEASES.length; index++) {
      expect(compareVersions(RELEASES[index - 1].version!, RELEASES[index].version!)).toBeGreaterThan(0);
    }
  });
});

describe("fragments", () => {
  it("parses plain and block bodies", () => {
    expect(parseFragment("kind: Added\nbody: Something new\ntime: 2026-11-02T00:00:00Z\n")).toEqual({ kind: "Added", body: "Something new" });
    expect(parseFragment("kind: Fixed\nbody: |\n  Line one\n  line two\ntime: x\n")).toEqual({ kind: "Fixed", body: "Line one line two" });
    expect(parseFragment("kind: Fixed\n")).toBeNull();
  });

  it("groups fragments under Unreleased", () => {
    expect(unreleasedRelease([])).toBeNull();
    expect(unreleasedRelease(["kind: Fixed\nbody: b\n", "kind: Added\nbody: a\n"])).toEqual({
      version: null,
      date: null,
      sections: [
        { kind: "Added", entries: ["a"] },
        { kind: "Fixed", entries: ["b"] },
      ],
    });
  });
});

describe("compareVersions", () => {
  it("orders semver numerically, with or without v", () => {
    expect(compareVersions("0.10.0", "0.9.9")).toBeGreaterThan(0);
    expect(compareVersions("v1.2.3", "1.2.3")).toBe(0);
    expect(compareVersions("1.0.0-rc.1", "1.0.0")).toBeLessThan(0);
    expect(compareVersions("1.0.0-rc.10", "1.0.0-rc.2")).toBeGreaterThan(0);
  });
});

describe("decideWhatsNew", () => {
  const releases = [release("0.1.0"), release("0.1.1"), release("0.2.0"), release("0.3.0")];

  it("records a first launch silently", () => {
    expect(decideWhatsNew("0.2.0", null, releases)).toEqual({ show: false, record: "0.2.0" });
  });

  it("does nothing when the version did not change (dev rebuilds)", () => {
    expect(decideWhatsNew("0.2.0", "0.2.0", releases)).toEqual({ show: false, record: null });
  });

  it("shows every release after the last seen one up to the current one, newest first", () => {
    const decision = decideWhatsNew("0.3.0", "0.1.0", releases);
    expect(decision.show && decision.releases.map((item) => item.version)).toEqual(["0.3.0", "0.2.0", "0.1.1"]);
  });

  it("records without showing on a downgrade or when no notes are embedded", () => {
    expect(decideWhatsNew("0.1.0", "0.2.0", releases)).toEqual({ show: false, record: "0.1.0" });
    expect(decideWhatsNew("0.4.0", "0.3.0", releases)).toEqual({ show: false, record: "0.4.0" });
  });

  it("ignores versions it cannot parse", () => {
    expect(decideWhatsNew("abc1234", "0.1.0", releases)).toEqual({ show: false, record: null });
    expect(decideWhatsNew("0.2.0", "garbage", releases)).toEqual({ show: false, record: "0.2.0" });
  });
});

describe("last seen version", () => {
  beforeEach(() => window.localStorage.clear());
  it("round-trips through localStorage without the v", () => {
    expect(lastSeenVersion()).toBeNull();
    recordSeenVersion("v0.2.0");
    expect(lastSeenVersion()).toBe("0.2.0");
  });
});
