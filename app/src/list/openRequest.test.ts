import { expect, test } from "vitest";

import { parseOpenRequest } from "./openRequest";

test("an open request is a list view and an optional pull request", () => {
  expect(parseOpenRequest({ view: "mine", pr_id: 12 })).toEqual({ view: "mine", prId: 12 });
  expect(parseOpenRequest({ view: "to_review" })).toEqual({ view: "to_review", prId: null });
  expect(parseOpenRequest({ view: "settings", pr_id: 1 })).toBeNull();
  expect(parseOpenRequest(null)).toBeNull();
});
