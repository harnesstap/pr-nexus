import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "../api";
import { PAGE_SIZE, dayBounds, type PageDto, type PageEntry, type PageRequest } from "./paging";

/** What decides which rows match; a change starts over from the first page. */
export type PageParams = Omit<PageRequest, "cursor" | "limit" | "now" | "day_start" | "week_start">;

export interface PagedList {
  /** Loaded entries, in stream order. */
  entries: PageEntry[];
  /** The last response; null until the first page arrives. */
  page: Omit<PageDto, "entries"> | null;
  /** First load for these params failed. */
  error: string | null;
  /** A request for new params is in flight (first load, a search, a filter). */
  pending: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  loadMore: () => void;
  retry: () => void;
}

/**
 * Infinite scroll over `list_page` (§8.2). Each request carries a sequence
 * number; a response for an older one is dropped, so a slow search cannot
 * overwrite a newer one. New params start over at the first page. A `version`
 * bump (sync, link, Slack send) re-reads everything already loaded in one call
 * and swaps it in place, so the scroll position holds.
 */
export function usePagedList(loadPage: (request: PageRequest) => Promise<PageDto>, params: PageParams, version: number, now: Date): PagedList {
  const [entries, setEntries] = useState<PageEntry[]>([]);
  const [page, setPage] = useState<Omit<PageDto, "entries"> | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The params (and retry attempt) the shown page answers; pending until it matches.
  const [answered, setAnswered] = useState<string | null>(null);
  // The cursor a next-page request is reading from, if any.
  const [moreFrom, setMoreFrom] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const seq = useRef(0);
  const loaded = useRef(0);
  const nowRef = useRef(now);
  useEffect(() => {
    nowRef.current = now;
  }, [now]);
  const key = JSON.stringify(params);
  const lastKey = useRef<string | null>(null);
  const want = `${attempt}:${key}`;
  const pending = answered !== want;

  const request = useCallback(
    (cursor: string | null, limit: number): PageRequest => {
      const at = nowRef.current;
      return { ...(JSON.parse(key) as PageParams), cursor, limit, now: at.toISOString(), ...dayBounds(at) };
    },
    [key],
  );

  useEffect(() => {
    const fresh = lastKey.current !== want;
    lastKey.current = want;
    const mine = ++seq.current;
    // A refresh keeps what is on screen and re-reads it all at once.
    const limit = fresh ? PAGE_SIZE : Math.max(PAGE_SIZE, loaded.current);
    loadPage(request(null, limit))
      .then((next) => {
        if (mine !== seq.current) {
          return;
        }
        const { entries: rows, ...rest } = next;
        loaded.current = rows.length;
        setEntries(rows);
        setPage(rest);
        setError(null);
        setMoreFrom(null);
        setAnswered(want);
      })
      .catch((err: unknown) => {
        if (mine !== seq.current) {
          return;
        }
        setError(errorMessage(err));
        setAnswered(want);
      });
  }, [loadPage, want, version, request]);

  const cursor = page?.next_cursor ?? null;
  const loadingMore = moreFrom !== null && moreFrom === cursor;
  const loadMore = useCallback(() => {
    if (!cursor || loadingMore || pending) {
      return;
    }
    const mine = seq.current;
    setMoreFrom(cursor);
    const done = () => setMoreFrom((current) => (current === cursor ? null : current));
    loadPage(request(cursor, PAGE_SIZE))
      .then((next) => {
        if (mine !== seq.current) {
          done();
          return;
        }
        const { entries: rows, ...rest } = next;
        setEntries((current) => {
          const merged = [...current, ...rows];
          loaded.current = merged.length;
          return merged;
        });
        setPage(rest);
        done();
      })
      .catch(done);
  }, [cursor, loadingMore, pending, loadPage, request]);

  const retry = useCallback(() => {
    setAttempt((value) => value + 1);
  }, []);

  return { entries, page, error, pending, loadingMore, hasMore: cursor !== null, loadMore, retry };
}
