import type { ReactNode } from "react";
import { Tooltip } from "../Tooltip";

/** Common shortcodes; unknown ones stay as `:name:`, like Slack without the emoji. */
export const EMOJI: Record<string, string> = {
  pray: "🙏",
  eyes: "👀",
  rocket: "🚀",
  tada: "🎉",
  wave: "👋",
  fire: "🔥",
  "+1": "👍",
  thumbsup: "👍",
  "-1": "👎",
  white_check_mark: "✅",
  heavy_check_mark: "✔️",
  x: "❌",
  warning: "⚠️",
  rotating_light: "🚨",
  hourglass: "⌛",
  hourglass_flowing_sand: "⏳",
  bell: "🔔",
  memo: "📝",
  link: "🔗",
  mag: "🔍",
  point_right: "👉",
  arrow_right: "➡️",
  bulb: "💡",
  thinking_face: "🤔",
  smile: "😄",
  slightly_smiling_face: "🙂",
  sweat_smile: "😅",
  raised_hands: "🙌",
  clap: "👏",
  red_circle: "🔴",
  large_green_circle: "🟢",
  large_yellow_circle: "🟡",
  sos: "🆘",
  ship: "🚢",
  heart: "❤️",
};

/** One piece of a rendered line. */
export type Span =
  | { kind: "text"; text: string }
  | { kind: "link"; text: string; url: string }
  | { kind: "mention"; text: string }
  | { kind: "bold" | "italic" | "strike"; children: Span[] }
  | { kind: "code"; text: string }
  | { kind: "emoji"; text: string; name: string };

/** A line, and whether it is a `>` quote. */
export interface Line {
  quote: boolean;
  spans: Span[];
}

const decode = (text: string) => text.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

// Order matters: angle-bracket entities, code, links, then emphasis and emoji.
const TOKEN = new RegExp(
  [
    "<([^<>]+)>", // 1: <url|label>, <#C|name>, <@U|name>, <!here>, <!subteam^S|@x>
    "`([^`\\n]+)`", // 2: inline code
    "\\[([^\\]\\n]+)\\]\\((https?://[^)\\s]+)\\)", // 3,4: [text](url), which Slack's markdown turns into a link
    "(https?://[^\\s<>]+[^\\s<>.,;:!?)\\]])", // 5: bare URL, auto-linked
    "\\*\\*([^*\\n]+)\\*\\*", // 6: **bold**
    "(?<![\\w*])\\*([^*\\n]+)\\*(?![\\w*])", // 7: *bold*
    "(?<![\\w_])_([^_\\n]+)_(?![\\w_])", // 8: _italic_
    "(?<![\\w~])~([^~\\n]+)~(?![\\w~])", // 9: ~strike~
    ":([a-z0-9_+-]+):", // 10: :emoji:
  ].join("|"),
  "g",
);

function angle(inner: string): Span {
  const [head, label] = inner.split("|", 2) as [string, string | undefined];
  if (head.startsWith("#")) {
    return { kind: "mention", text: `#${label ?? head.slice(1)}` };
  }
  if (head.startsWith("@")) {
    return { kind: "mention", text: `@${label ?? head.slice(1)}` };
  }
  if (head.startsWith("!subteam^")) {
    return { kind: "mention", text: label ?? `@${head.slice("!subteam^".length)}` };
  }
  if (head.startsWith("!")) {
    return { kind: "mention", text: `@${head.slice(1)}` };
  }
  return { kind: "link", url: decode(head), text: decode(label ?? head) };
}

/** Slack mrkdwn → spans, without HTML: React escapes everything it renders. */
export function parseInline(text: string): Span[] {
  const out: Span[] = [];
  let last = 0;
  for (const match of text.matchAll(TOKEN)) {
    const at = match.index ?? 0;
    if (at > last) {
      out.push({ kind: "text", text: decode(text.slice(last, at)) });
    }
    const [whole, ang, code, mdText, mdUrl, bare, bold2, bold, italic, strike, emoji] = match;
    if (ang !== undefined) {
      out.push(angle(ang));
    } else if (code !== undefined) {
      out.push({ kind: "code", text: decode(code) });
    } else if (mdText !== undefined && mdUrl !== undefined) {
      out.push({ kind: "link", text: decode(mdText), url: mdUrl });
    } else if (bare !== undefined) {
      out.push({ kind: "link", text: decode(bare), url: decode(bare) });
    } else if (bold2 !== undefined || bold !== undefined) {
      out.push({ kind: "bold", children: parseInline(bold2 ?? bold ?? "") });
    } else if (italic !== undefined) {
      out.push({ kind: "italic", children: parseInline(italic) });
    } else if (strike !== undefined) {
      out.push({ kind: "strike", children: parseInline(strike) });
    } else if (emoji !== undefined) {
      const glyph = EMOJI[emoji];
      out.push(glyph ? { kind: "emoji", text: glyph, name: emoji } : { kind: "text", text: whole });
    }
    last = at + whole.length;
  }
  if (last < text.length) {
    out.push({ kind: "text", text: decode(text.slice(last)) });
  }
  // An unknown :shortcode: stays text: join it to its neighbours.
  return out.reduce<Span[]>((joined, span) => {
    const prev = joined[joined.length - 1];
    if (span.kind === "text" && prev?.kind === "text") {
      joined[joined.length - 1] = { kind: "text", text: prev.text + span.text };
    } else {
      joined.push(span);
    }
    return joined;
  }, []);
}

export function parseMrkdwn(text: string): Line[] {
  return text
    .replace(/\n+$/, "")
    .split("\n")
    .map((line) => {
      const quote = /^(>|&gt;) ?/.exec(line);
      return { quote: quote !== null, spans: parseInline(quote ? line.slice(quote[0].length) : line) };
    });
}

function renderSpans(spans: Span[], onOpen: (url: string) => void): ReactNode[] {
  return spans.map((span, index) => {
    switch (span.kind) {
      case "text":
        return span.text;
      case "link":
        return (
          <a
            key={index}
            href={span.url}
            onClick={(event) => {
              event.preventDefault();
              onOpen(span.url);
            }}
            className="text-sky-300 hover:underline"
          >
            {span.text}
          </a>
        );
      case "mention":
        return (
          <span key={index} className="rounded bg-sky-500/15 px-0.5 text-sky-200">
            {span.text}
          </span>
        );
      case "bold":
        return <strong key={index}>{renderSpans(span.children, onOpen)}</strong>;
      case "italic":
        return <em key={index}>{renderSpans(span.children, onOpen)}</em>;
      case "strike":
        return <s key={index}>{renderSpans(span.children, onOpen)}</s>;
      case "code":
        return (
          <code key={index} className="rounded border border-slate-700 bg-slate-800 px-1 text-[0.85em] text-rose-200">
            {span.text}
          </code>
        );
      case "emoji":
        return (
          <Tooltip key={index} label={`:${span.name}:`}>
            <span role="img" aria-label={span.name}>
              {span.text}
            </span>
          </Tooltip>
        );
      default: {
        const never: never = span;
        return never;
      }
    }
  });
}

/** A message laid out the way Slack shows it: links by their text, bold, emoji, quotes. */
export function Mrkdwn({ text, onOpen, label }: { text: string; onOpen: (url: string) => void; label?: string }) {
  return (
    <div aria-label={label} className="space-y-0 text-[13px] leading-relaxed text-slate-100">
      {parseMrkdwn(text).map((line, index) =>
        line.quote ? (
          <blockquote key={index} className="border-l-4 border-slate-600 pl-2 text-slate-300">
            {renderSpans(line.spans, onOpen)}
          </blockquote>
        ) : (
          <p key={index} className="min-h-[1.25em] whitespace-pre-wrap break-words">
            {renderSpans(line.spans, onOpen)}
          </p>
        ),
      )}
    </div>
  );
}
