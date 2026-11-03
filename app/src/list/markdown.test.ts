import { describe, expect, it } from "vitest";
import { escapeLinkText, prsToMarkdown } from "./markdown";

describe("prsToMarkdown", () => {
  it("formats a single PR with a trailing newline", () => {
    expect(prsToMarkdown([{ title: "Fix login", url: "https://github.com/acme/web/pull/1" }])).toBe("- [Fix login](https://github.com/acme/web/pull/1)\n");
  });

  it("keeps the given order for several PRs", () => {
    const text = prsToMarkdown([
      { title: "Second", url: "https://github.com/acme-labs/api/pull/9" },
      { title: "First", url: "https://github.com/acme/web/pull/2" },
    ]);
    expect(text).toBe("- [Second](https://github.com/acme-labs/api/pull/9)\n- [First](https://github.com/acme/web/pull/2)\n");
  });

  it("escapes brackets and trims titles", () => {
    expect(prsToMarkdown([{ title: "  [WIP] add [beta] flag \n", url: "https://github.com/acme/web/pull/3" }])).toBe(
      "- [\\[WIP\\] add \\[beta\\] flag](https://github.com/acme/web/pull/3)\n",
    );
    expect(escapeLinkText("a\\b")).toBe("a\\\\b");
  });

  it("returns nothing for no PRs", () => {
    expect(prsToMarkdown([])).toBe("");
  });
});
