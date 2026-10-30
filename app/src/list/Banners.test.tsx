import { beforeEach, expect, test } from "vitest";
import { showMappingHint, dismissMappingHint } from "./hints";

beforeEach(() => window.localStorage.clear());

test("the mapping hint needs Slack, no rows, and no dismissal", () => {
  expect(showMappingHint(0, 1)).toBe(true);
  expect(showMappingHint(3, 1)).toBe(false);
  expect(showMappingHint(0, 0)).toBe(false);
  dismissMappingHint();
  expect(showMappingHint(0, 1)).toBe(false);
});
