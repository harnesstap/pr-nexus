import { expect, test } from "vitest";
import { mapLimit } from "./mapLimit";

test("runs at most `limit` tasks at once, keeps order and collects failures", async () => {
  let inFlight = 0;
  let peak = 0;
  const results = await mapLimit([1, 2, 3, 4, 5, 6, 7], 3, async (n) => {
    inFlight++;
    peak = Math.max(peak, inFlight);
    await new Promise((resolve) => setTimeout(resolve, 8 - n));
    inFlight--;
    if (n === 4) {
      throw new Error("boom");
    }
    return n * 10;
  });
  expect(peak).toBe(3);
  expect(results.map((r) => (r.status === "fulfilled" ? r.value : String(r.reason)))).toEqual([10, 20, 30, "Error: boom", 50, 60, 70]);
});

test("empty input resolves at once", async () => {
  expect(await mapLimit([], 4, () => Promise.resolve(1))).toEqual([]);
});
