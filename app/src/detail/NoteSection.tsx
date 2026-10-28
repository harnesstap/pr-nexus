import { useEffect, useRef, useState } from "react";
import type { Api } from "../api";
import { errorMessage } from "../api";
import { ago } from "../list/model";
import { plainText } from "../textInput";

type Saved = { note: string; updatedAt: string | null };

/** A private note on the PR. Saves on blur, on ⌘/Ctrl+Enter or with Save. */
export function NoteSection({ api, prId }: { api: Api; prId: number }) {
  const [saved, setSaved] = useState<Saved | null>(null);
  const [draft, setDraft] = useState("");
  const [status, setStatus] = useState<"idle" | "saving" | "error">("idle");
  const [error, setError] = useState<string | null>(null);
  const saving = useRef<Promise<void> | null>(null);

  useEffect(() => {
    let cancel = false;
    api
      .prNote(prId)
      .then((note) => {
        if (!cancel) {
          setSaved({ note: note.note, updatedAt: note.updated_at });
          setDraft(note.note);
        }
      })
      .catch((err: unknown) => !cancel && setError(errorMessage(err)));
    return () => {
      cancel = true;
    };
  }, [api, prId]);

  const dirty = saved !== null && draft !== saved.note;

  // Switching PRs unmounts the panel before a blur: keep what was typed.
  const latest = useRef({ dirty, draft });
  useEffect(() => {
    latest.current = { dirty, draft };
  }, [dirty, draft]);
  useEffect(
    () => () => {
      if (latest.current.dirty && !saving.current) {
        void api.setPrNote(prId, latest.current.draft).catch(() => undefined);
      }
    },
    [api, prId],
  );

  async function save() {
    if (!dirty || saving.current) {
      return;
    }
    setStatus("saving");
    const run = api
      .setPrNote(prId, draft)
      .then((note) => {
        setSaved({ note: note.note, updatedAt: note.updated_at });
        setDraft((current) => (current === draft ? note.note : current));
        setStatus("idle");
        setError(null);
      })
      .catch((err: unknown) => {
        setStatus("error");
        setError(errorMessage(err));
      })
      .finally(() => {
        saving.current = null;
      });
    saving.current = run;
    await run;
  }

  return (
    <section aria-label="Note" className="rounded-lg border border-slate-800 bg-slate-950/40 p-3">
      <div className="mb-1.5 flex items-center gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-400">Note</h3>
        <span className="flex-1" />
        <span role="status" className="text-[11px] text-slate-500">
          {status === "saving" ? "Saving…" : dirty ? "Unsaved" : saved?.updatedAt ? `Saved ${ago(saved.updatedAt, new Date())}` : ""}
        </span>
        <button
          type="button"
          className="rounded px-2 py-0.5 text-xs text-violet-300 hover:bg-slate-800 disabled:text-slate-600 disabled:hover:bg-transparent"
          aria-label="Save note"
          disabled={!dirty || status === "saving"}
          onClick={() => void save()}
        >
          Save
        </button>
      </div>
      <textarea
        {...plainText}
        aria-label="Note for this pull request"
        rows={3}
        placeholder={saved === null ? "Loading…" : "Only you see this. Saved when you leave the field."}
        disabled={saved === null}
        className="w-full resize-y rounded border border-slate-700 bg-slate-900 px-2 py-1.5 text-sm outline-none placeholder:text-slate-600 focus:border-violet-400"
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => void save()}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void save();
          }
        }}
      />
      {error && <p className="mt-1 text-xs text-rose-300">{error}</p>}
    </section>
  );
}
