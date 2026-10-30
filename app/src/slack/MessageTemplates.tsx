import { useEffect, useState } from "react";
import { errorMessage, type Api, type MessageTemplate, type TemplateKind } from "../api";
import { plainText } from "../textInput";
import { Mrkdwn } from "./mrkdwn";

const LABELS: Record<TemplateKind, string> = {
  single_request: "Single request",
  batch_request: "Batch request",
  single_remind: "Single reminder",
  batch_remind: "Batch reminder",
  item: "Batch item",
};

const ORDER: TemplateKind[] = ["single_request", "batch_request", "single_remind", "batch_remind", "item"];

const HINTS: Record<TemplateKind, string> = {
  single_request: "Must contain {url}.",
  single_remind: "Must contain {url}.",
  batch_request: "Must contain {items}.",
  batch_remind: "Must contain {items}.",
  item: "Repeated once per PR in {items}. Must contain {url}.",
};

/** Settings → Slack → Message templates (§7.6). Each template saves on its own. */
export function MessageTemplates({ api, onOpen = () => undefined }: { api: Api; onOpen?: (url: string) => void }) {
  const [templates, setTemplates] = useState<MessageTemplate[] | null>(null);
  useEffect(() => {
    let live = true;
    api.messageTemplates?.().then(
      (list) => live && setTemplates(list),
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [api]);
  if (!templates) {
    return null;
  }
  const byKind = new Map(templates.map((template) => [template.kind, template]));
  return (
    <section className="mt-8" aria-labelledby="message-templates">
      <h3 id="message-templates" className="text-base font-semibold">
        Message templates
      </h3>
      <p className="mb-3 mt-1 text-xs text-slate-500">
        Placeholders: <code>{"{title}"}</code> (shown as inline code), <code>{"{url}"}</code>, <code>{"{who}"}</code> (“ (requester)” or
        nothing), <code>{"{requester}"}</code>, <code>{"{count}"}</code>, and <code>{"{items}"}</code> in batch templates. Write{" "}
        <code>{"{{"}</code> and <code>{"}}"}</code> for literal braces. Your optional note is added after a blank line.
      </p>
      <div className="space-y-6">
        {ORDER.map((kind) => {
          const template = byKind.get(kind);
          return template ? <TemplateEditor key={kind} api={api} template={template} onOpen={onOpen} /> : null;
        })}
      </div>
    </section>
  );
}

function TemplateEditor({ api, template, onOpen }: { api: Api; template: MessageTemplate; onOpen: (url: string) => void }) {
  const [saved, setSaved] = useState(template.text);
  const [text, setText] = useState(template.text);
  const [preview, setPreview] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const label = LABELS[template.kind];
  const id = `template-${template.kind}`;

  useEffect(() => {
    let live = true;
    const timer = setTimeout(() => {
      api.previewMessageTemplate?.(template.kind, text).then(
        (next) => {
          if (live) {
            setPreview(next);
            setError(null);
          }
        },
        (err: unknown) => live && setError(errorMessage(err)),
      );
    }, 150);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [api, template.kind, text]);

  const save = async (next: string | null) => {
    setBusy(true);
    try {
      await api.saveMessageTemplate?.(template.kind, next);
      const value = next ?? template.default_text;
      setSaved(value);
      setText(value);
      setError(null);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rounded border border-slate-800 p-3">
      <div className="mb-1 flex items-center justify-between gap-2">
        <label htmlFor={id} className="text-sm font-medium">
          {label}
        </label>
        <div className="flex gap-2 text-xs">
          <button
            type="button"
            className="rounded border border-slate-700 px-2 py-0.5 text-slate-300 hover:bg-slate-800 disabled:opacity-40"
            disabled={busy || (saved === template.default_text && text === template.default_text)}
            onClick={() => void save(null)}
            aria-label={`Reset ${label} to default`}
          >
            Reset to default
          </button>
          <button
            type="button"
            className="rounded bg-violet-600 px-2 py-0.5 text-white hover:bg-violet-500 disabled:opacity-40"
            disabled={busy || text === saved || error !== null}
            onClick={() => void save(text)}
            aria-label={`Save ${label}`}
          >
            Save
          </button>
        </div>
      </div>
      <textarea
        {...plainText}
        id={id}
        rows={Math.max(3, text.split("\n").length)}
        className="w-full rounded border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-xs"
        value={text}
        aria-invalid={error !== null}
        aria-describedby={`${id}-hint`}
        onChange={(event) => setText(event.target.value)}
      />
      <p id={`${id}-hint`} className="mt-1 text-xs text-slate-500">
        {HINTS[template.kind]}
      </p>
      {error ? (
        <p role="alert" className="mt-2 text-xs text-rose-400">
          {error}
        </p>
      ) : (
        <div className="mt-2 rounded border border-slate-800 bg-slate-950 p-3">
          <Mrkdwn label={`${label} preview`} text={preview} onOpen={onOpen} />
        </div>
      )}
    </div>
  );
}
