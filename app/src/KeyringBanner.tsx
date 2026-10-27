import { useEffect, useRef, useState } from "react";
import type { Api, KeyringState, KeyringStatus } from "./api";

/** Keychain states that can still change without the user doing anything. */
function pending(state: KeyringState): boolean {
  switch (state) {
    case "waiting":
    case "timed_out":
      return true;
    case "ready":
    case "denied":
    case "no_backend":
    case "error":
    case "disabled":
      return false;
    default: {
      const unknown: never = state;
      return unknown;
    }
  }
}

interface Props {
  api: Pick<Api, "keyringStatus" | "keyringRetry">;
  /** From bootstrap; omitted on the loading screen, which asks once. */
  initial?: KeyringStatus;
  /** The keychain became ready after it was not (tokens were read or saved). */
  onReady?: () => void;
  pollMs?: number;
}

/**
 * The keychain notice. While a system prompt is pending it shows a calm hint
 * and polls; a real failure shows its reason and, when it can help, Retry.
 * It never blocks the UI: every call returns without waiting on the keychain.
 */
export function KeyringBanner({ api, initial, onReady, pollMs = 1000 }: Props) {
  const [status, setStatus] = useState<KeyringStatus | null>(initial ?? null);
  const [retrying, setRetrying] = useState(false);
  const blocked = useRef(initial ? initial.state !== "ready" : true);
  const readyCallback = useRef(onReady);
  useEffect(() => {
    readyCallback.current = onReady;
  }, [onReady]);

  const state = status?.state ?? null;
  useEffect(() => {
    if (state !== null && !pending(state)) {
      return;
    }
    let cancel = false;
    const check = () =>
      void api
        .keyringStatus()
        .then((next) => {
          if (!cancel) {
            setStatus(next);
          }
        })
        .catch(() => undefined);
    if (state === null) {
      check();
    }
    const timer = window.setInterval(check, pollMs);
    return () => {
      cancel = true;
      window.clearInterval(timer);
    };
  }, [api, state, pollMs]);

  useEffect(() => {
    if (state === "ready" && blocked.current) {
      blocked.current = false;
      readyCallback.current?.();
    } else if (state !== null && state !== "ready") {
      blocked.current = true;
    }
  }, [state]);

  if (!status || status.state === "ready" || !status.message) {
    return null;
  }
  const waiting = status.state === "waiting";
  const retry = () => {
    setRetrying(true);
    void api
      .keyringRetry()
      .then(setStatus)
      .catch(() => undefined)
      .finally(() => setRetrying(false));
  };
  return (
    <div
      role={waiting ? "status" : "alert"}
      data-keyring-state={status.state}
      className={
        waiting
          ? "flex flex-wrap items-center gap-3 border-b border-sky-500/40 bg-sky-500/10 px-4 py-2 text-sm text-sky-100"
          : "flex flex-wrap items-center gap-3 border-b border-amber-500/40 bg-amber-500/10 px-4 py-2 text-sm text-amber-100"
      }
    >
      <span>{status.message}</span>
      {status.can_retry && (
        <button type="button" className="underline disabled:opacity-60" disabled={retrying} onClick={retry}>
          {retrying ? "Retrying…" : "Retry keychain access"}
        </button>
      )}
    </div>
  );
}
