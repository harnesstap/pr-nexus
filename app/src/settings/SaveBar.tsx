import { useEffect, useState } from "react";
import { errorMessage } from "../api";

/** How long "Saved" stays after a save. */
export const SAVED_MS = 2_500;

interface Props {
  /** Unsaved changes exist: the bar is shown only then. */
  dirty: boolean;
  saving: boolean;
  error: string | null;
  onSave: () => void;
  onDiscard: () => void;
  /** Bumped after each successful save; shows a short "Saved" toast. */
  savedAt: number;
  /** Inside a form: Save submits it, so field constraints are checked first. */
  submit?: boolean;
}

/**
 * The one Settings save pattern: edits stay local until Save in a sticky bar
 * at the bottom, shown only while something is unsaved. Saving shows "Saved".
 */
export function SaveBar({ dirty, saving, error, onSave, onDiscard, savedAt, submit = false }: Props) {
  const [hidden, setHidden] = useState(0);
  const toast = savedAt !== hidden;
  useEffect(() => {
    const timer = window.setTimeout(() => setHidden(savedAt), SAVED_MS);
    return () => window.clearTimeout(timer);
  }, [savedAt]);

  if (!dirty) {
    return toast ? (
      <p role="status" className="sticky bottom-4 ml-auto mt-6 w-fit rounded border border-emerald-700 bg-slate-900 px-3 py-1.5 text-sm text-emerald-200 shadow-lg">
        Saved
      </p>
    ) : null;
  }
  return (
    <div
      role="region"
      aria-label="Unsaved changes"
      className="sticky bottom-4 z-10 mt-6 flex items-center gap-3 rounded-lg border border-slate-700 bg-slate-900/95 px-4 py-2 text-sm shadow-lg"
    >
      <span className="text-slate-300">{error ? <span role="alert" className="text-rose-300">{error}</span> : "You have unsaved changes."}</span>
      <span className="ml-auto flex gap-2">
        <button type="button" className="rounded px-3 py-1.5 text-slate-300 hover:bg-slate-800 disabled:opacity-50" disabled={saving} onClick={onDiscard}>
          Discard
        </button>
        <button
          type={submit ? "submit" : "button"}
          className="rounded bg-violet-500 px-3 py-1.5 font-medium text-white disabled:opacity-50"
          disabled={saving}
          onClick={submit ? undefined : onSave}
        >
          {saving ? "Saving…" : "Save"}
        </button>
      </span>
    </div>
  );
}

/** Draft state for a Settings form: `dirty`, `save` and `discard` for the SaveBar. */
export function useSaveBar<T>(saved: T, persist: (draft: T, saved: T) => Promise<T | void>) {
  const [base, setBase] = useState(saved);
  const [draft, setDraft] = useState(saved);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState(0);
  const dirty = JSON.stringify(draft) !== JSON.stringify(base);
  async function save() {
    setSaving(true);
    setError(null);
    try {
      const next = (await persist(draft, base)) ?? draft;
      setBase(next);
      setDraft(next);
      setSavedAt((value) => value + 1);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  }
  return {
    draft,
    setDraft: (next: T | ((current: T) => T)) => {
      setError(null);
      setDraft(next);
    },
    dirty,
    bar: {
      dirty,
      saving,
      error,
      savedAt,
      onSave: () => void save(),
      onDiscard: () => {
        setError(null);
        setDraft(base);
      },
    },
  };
}
