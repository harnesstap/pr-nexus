import type { BotReviewNote, BotReviews as BotReviewsData } from "../api";

interface Props {
  reviews: BotReviewsData | undefined;
  onOpen: (url: string) => void;
}

function noteKindLabel(kind: BotReviewNote["kind"]): string {
  switch (kind) {
    case "issue":
      return "Comment";
    case "thread":
      return "Review comment";
    case "review":
      return "Review";
    default: {
      const unexpected: never = kind;
      return unexpected;
    }
  }
}

/** Bot comments and reviews, kept out of the human comment counts. */
export function BotReviews({ reviews, onOpen }: Props) {
  if (!reviews || reviews.count === 0) {
    return null;
  }
  const unresolved =
    reviews.unresolved === 1 ? "1 unresolved" : `${reviews.unresolved} unresolved`;
  return (
    <section aria-label="Bot reviews" className="rounded-lg border border-slate-800 bg-slate-950/40 p-3">
      <div className="flex min-w-0 items-baseline gap-2">
        <h3 className="text-sm font-medium text-slate-200">Bot reviews</h3>
        <p className="truncate text-xs text-slate-500">
          {reviews.count} {reviews.count === 1 ? "comment" : "comments"} · {unresolved}
        </p>
      </div>
      {reviews.latest_snippet && (
        <p className="mt-1 line-clamp-2 text-sm text-slate-300">{reviews.latest_snippet}</p>
      )}
      <ul className="mt-2 divide-y divide-slate-800/80">
        {reviews.notes.map((note, index) => (
          <li key={`${note.url}-${note.at}-${index}`} className="py-1.5 first:pt-0 last:pb-0">
            <button
              type="button"
              className="block w-full min-w-0 text-left text-sm enabled:hover:underline disabled:cursor-default"
              disabled={note.url.length === 0}
              onClick={() => onOpen(note.url)}
            >
              <span className="text-slate-400">{note.login}</span>
              <span className="text-slate-600"> · {noteKindLabel(note.kind)}</span>
              {note.unresolved && <span className="text-slate-500"> · unresolved</span>}
              {note.snippet && <span className="mt-0.5 block truncate text-slate-300">{note.snippet}</span>}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
