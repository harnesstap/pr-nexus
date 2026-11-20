import { expect, test } from "vitest";
import { exactPrLink, githubQueryOf, linkMatches, parsePrLink, sortLocksOrder, splitSearch, suggestionsFor } from "./searchText";

test("qualifiers go to GitHub and the other words stay local", () => {
  const parts = splitSearch('author:@me passkeys -label:"bug fix" created:>@today-1w "two words" repo:acme/widgets sort:updated-desc');
  expect(parts.github).toBe('author:@me -label:"bug fix" created:>@today-1w repo:acme/widgets sort:updated-desc');
  expect(parts.text).toBe('passkeys "two words"');
  const bare = splitSearch("draft:true draft is:draft");
  expect(bare.github).toBe("draft:true is:draft");
  expect(bare.text).toBe("draft");
});

test("a pull request URL is a link, and other pasted text stays local", () => {
  const parts = splitSearch("https://github.com/acme/widgets/pull/1 fix(api): #123");
  expect(parts.github).toBe("");
  expect(parts.links).toEqual([{ owner: "acme", name: "widgets", number: 1 }]);
  expect(parts.text).toBe("fix(api): #123");
  expect(exactPrLink("https://github.com/acme/widgets/pull/1 fix(api): #123")).toBeNull();
  const other = splitSearch("https://example.com/acme/widgets/pull/1 notes");
  expect(other.links).toEqual([]);
  expect(other.text).toBe("https://example.com/acme/widgets/pull/1 notes");
  // An unfinished quoted qualifier is still a qualifier, so the server can reject it.
  expect(splitSearch('label:"bug').github).toBe('label:"bug');
});

test("pull request links accept a URL, a trailing path, and owner/repo#n", () => {
  const url = parsePrLink("https://github.com/Acme/Widgets/pull/332/files?w=1#diff");
  expect(url).toEqual({ owner: "Acme", name: "Widgets", number: 332 });
  expect(parsePrLink("github.com/acme/widgets/pull/332")).toEqual({ owner: "acme", name: "widgets", number: 332 });
  expect(parsePrLink("acme/widgets#332")).toEqual({ owner: "acme", name: "widgets", number: 332 });
  expect(exactPrLink("  ACME/Widgets#332  ")).toEqual({ owner: "ACME", name: "Widgets", number: 332 });
  expect(parsePrLink("https://github.com/acme/widgets/issues/332")).toBeNull();
  expect(parsePrLink("acme/widgets#0")).toBeNull();
  const link = { owner: "Acme", name: "Widgets", number: 332 };
  expect(linkMatches({ owner: "acme", repo: "acme/widgets", number: 332 }, link)).toBe(true);
  expect(linkMatches({ owner: "acme", repo: "acme/widgets", number: 333 }, link)).toBe(false);
});

test("only an explicit sort: qualifier locks the order", () => {
  expect(sortLocksOrder("author:me is:draft passkeys")).toBe(false);
  expect(sortLocksOrder("author:me sort:created-asc")).toBe(true);
  expect(sortLocksOrder('sort:"created-asc"')).toBe(true);
  expect(sortLocksOrder("-sort:updated-desc")).toBe(false);
});

test("an upper-case Sort: key locks the order like the GitHub query", () => {
  expect(sortLocksOrder("Sort:created-asc author:octo")).toBe(true);
  expect(sortLocksOrder("SORT:updated-desc")).toBe(true);
  expect(sortLocksOrder("-Sort:updated-desc")).toBe(false);
});

test("free text alone is not a GitHub query", () => {
  expect(githubQueryOf({ search: "passkeys", github: "" })).toBe("");
  expect(githubQueryOf({ search: "", github: "author:me" })).toBe("author:me");
  expect(githubQueryOf({ search: "author:me passkeys", github: "author:me" })).toBe("author:me");
  expect(githubQueryOf({ search: "is:draft review:approved author:ada", github: "" })).toBe("author:ada");
  expect(githubQueryOf({ search: "-org:acme org:acme", github: "" })).toBe("-org:acme");
});

test("suggestions follow the qualifier being typed", () => {
  expect(suggestionsFor("au").map((item) => item.label)).toEqual(["author:"]);
  expect(suggestionsFor("a").map((item) => item.label)).toContain("assignee:");
  expect(suggestionsFor("-is").map((item) => item.insert)).toEqual(["-is:"]);
  expect(suggestionsFor("is:d").map((item) => item.label)).toEqual(["is:draft"]);
  expect(suggestionsFor("is:draft")).toEqual([]);
  expect(suggestionsFor("bump").map((item) => item.label)).toEqual(["bump:"]);
  expect(suggestionsFor("bump:").map((item) => item.label)).toEqual(["bump:due"]);
  expect(suggestionsFor("bump:")[0]?.hint).toBe("");
  expect(suggestionsFor("passkeys")).toEqual([]);
  expect(suggestionsFor("")).toEqual([]);
});
