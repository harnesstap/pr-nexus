import { expect, test } from "vitest";
import { githubQueryOf, sortLocksOrder, splitSearch, suggestionsFor } from "./searchText";

test("qualifiers go to GitHub and the other words stay local", () => {
  const parts = splitSearch('author:@me passkeys -label:"bug fix" created:>@today-1w "two words" repo:acme/widgets sort:updated-desc');
  expect(parts.github).toBe('author:@me -label:"bug fix" created:>@today-1w repo:acme/widgets sort:updated-desc');
  expect(parts.text).toBe('passkeys "two words"');
  const bare = splitSearch("draft:true draft is:draft");
  expect(bare.github).toBe("draft:true is:draft");
  expect(bare.text).toBe("draft");
});

test("a pasted link and a plain word stay local", () => {
  const parts = splitSearch("https://github.com/acme/widgets/pull/1 fix(api): #123");
  expect(parts.github).toBe("");
  expect(parts.text).toBe("https://github.com/acme/widgets/pull/1 fix(api): #123");
  // An unfinished quoted qualifier is still a qualifier, so the server can reject it.
  expect(splitSearch('label:"bug').github).toBe('label:"bug');
});

test("only an explicit sort: qualifier locks the order", () => {
  expect(sortLocksOrder("author:me is:draft passkeys")).toBe(false);
  expect(sortLocksOrder("author:me sort:created-asc")).toBe(true);
  expect(sortLocksOrder('sort:"created-asc"')).toBe(true);
  expect(sortLocksOrder("-sort:updated-desc")).toBe(false);
});

test("free text alone is not a GitHub query", () => {
  expect(githubQueryOf({ search: "passkeys", github: "" })).toBe("");
  expect(githubQueryOf({ search: "", github: "author:me" })).toBe("author:me");
  expect(githubQueryOf({ search: "author:me passkeys", github: "author:me" })).toBe("author:me");
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
