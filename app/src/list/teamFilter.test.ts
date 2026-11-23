import { expect, test } from "vitest";
import { expandTeamFilters, teamFilterLabels, teamMatches, toggleTeamFilter } from "./teamFilter";

const known = [
  { login: "acme/platform", name: "Platform" },
  { login: "acme-labs/platform", name: "Platform" },
  { login: "acme/payments", name: "Payments" },
];

test("an org/slug filter matches only that team", () => {
  expect(teamMatches("acme/platform", "acme/platform")).toBe(true);
  expect(teamMatches("acme-labs/platform", "acme/platform")).toBe(false);
  expect(teamMatches("acme/platform", "acme/payments")).toBe(false);
});

test("a bare slug matches that slug in every org", () => {
  expect(teamMatches("acme/platform", "platform")).toBe(true);
  expect(teamMatches("acme-labs/platform", "Platform")).toBe(true);
  expect(teamMatches("acme/web-platform", "platform")).toBe(false);
});

test("a bare display name matches when the slug does not", () => {
  expect(teamMatches("acme/web-platform", "Web platform", "Web platform")).toBe(true);
  expect(teamMatches("acme/web-platform", "acme-labs/web-platform", "Web platform")).toBe(false);
});

test("expand rewrites an unambiguous slug and keeps both when it is shared", () => {
  expect(expandTeamFilters(["payments"], known)).toEqual(["acme/payments"]);
  expect(expandTeamFilters(["platform"], known)).toEqual(["acme/platform", "acme-labs/platform"]);
  expect(expandTeamFilters(["acme/platform"], known)).toEqual(["acme/platform"]);
  expect(expandTeamFilters(["missing"], known)).toEqual(["missing"]);
});

test("toggle expands a shared slug before dropping one org", () => {
  const options = [
    { slug: "acme/platform", name: "Platform", count: 1 },
    { slug: "acme-labs/platform", name: "Platform", count: 1 },
  ];
  expect(toggleTeamFilter(["platform"], "acme-labs/platform", options)).toEqual(["acme/platform"]);
  expect(toggleTeamFilter(["acme/platform"], "acme-labs/platform", options)).toEqual(["acme/platform", "acme-labs/platform"]);
});

test("ambiguous teams show the org; unique ones keep the current label", () => {
  const labels = teamFilterLabels([
    { slug: "acme/platform", name: "Platform", count: 2 },
    { slug: "acme-labs/platform", name: "Platform", count: 1 },
    { slug: "acme/payments", name: "Payments", count: 4 },
  ]);
  expect(labels).toEqual([
    { id: "acme/platform", label: "Platform", hint: "acme", countLabel: "(2)" },
    { id: "acme-labs/platform", label: "Platform", hint: "acme-labs", countLabel: "(1)" },
    { id: "acme/payments", label: "Payments · acme/payments (4)" },
  ]);
});
