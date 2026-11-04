/** Concurrent GitHub writes from one bulk action; the backend gate allows 4 too. */
export const BULK_CONCURRENCY = 4;

/**
 * Run `task` on every item with at most `limit` in flight. Results keep the
 * input order; a rejected task yields `{ ok: false }` instead of stopping the rest.
 */
export async function mapLimit<T, R>(
  items: readonly T[],
  limit: number,
  task: (item: T, index: number) => Promise<R>,
): Promise<PromiseSettledResult<R>[]> {
  const results: PromiseSettledResult<R>[] = new Array(items.length);
  let next = 0;
  async function worker() {
    while (next < items.length) {
      const index = next++;
      try {
        results[index] = { status: "fulfilled", value: await task(items[index], index) };
      } catch (reason) {
        results[index] = { status: "rejected", reason };
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(Math.max(1, limit), items.length) }, worker));
  return results;
}
