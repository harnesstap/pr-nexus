import { isPlainShortcut } from "./keys";

/**
 * The one list of keyboard shortcuts. The list's key handler, the `?` cheat
 * sheet and the tooltip/menu hints all read from here, so they cannot drift.
 */
export type ShortcutGroup = "Navigation" | "Selection" | "Actions";

export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = ["Navigation", "Selection", "Actions"];

export type ShortcutId =
  | "next"
  | "prev"
  | "details"
  | "search"
  | "palette"
  | "help"
  | "toggleRow"
  | "toggleSection"
  | "selectAll"
  | "clear"
  | "request"
  | "slack"
  | "github"
  | "filters"
  | "project"
  | "star";

type KeyEvent = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">;

export interface Shortcut {
  id: ShortcutId;
  group: ShortcutGroup;
  /** What the cheat sheet shows, one entry per alternative key. */
  keys: string[];
  label: string;
  /** Cmd (macOS) or Ctrl elsewhere. Plain shortcuts ignore Cmd, Ctrl and Alt. */
  mod?: boolean;
  /** Matched keys, lowercased for letters. */
  match: string[];
  /** Shift must be held (true), must not be (false), or either (undefined). */
  shift?: boolean;
}

export const SHORTCUTS: readonly Shortcut[] = [
  { id: "next", group: "Navigation", keys: ["j", "↓"], label: "Next PR", match: ["j", "ArrowDown"], shift: false },
  { id: "prev", group: "Navigation", keys: ["k", "↑"], label: "Previous PR", match: ["k", "ArrowUp"], shift: false },
  { id: "details", group: "Navigation", keys: ["Enter"], label: "Open PR details", match: ["Enter"] },
  { id: "search", group: "Navigation", keys: ["/"], label: "Search", match: ["/"] },
  { id: "palette", group: "Navigation", keys: ["Mod+K"], label: "Command palette", match: ["k"], mod: true },
  { id: "help", group: "Navigation", keys: ["?"], label: "Keyboard shortcuts", match: ["?"] },
  { id: "toggleRow", group: "Selection", keys: ["x"], label: "Select focused PR", match: ["x"], shift: false },
  { id: "toggleSection", group: "Selection", keys: ["Shift+x"], label: "Select focused group", match: ["x"], shift: true },
  { id: "selectAll", group: "Selection", keys: ["Mod+A"], label: "Select all", match: ["a"], mod: true },
  { id: "clear", group: "Selection", keys: ["Esc"], label: "Clear selection / close", match: ["Escape"] },
  { id: "request", group: "Actions", keys: ["r"], label: "Request review / bump on Slack", match: ["r"] },
  { id: "slack", group: "Actions", keys: ["s"], label: "Open Slack request", match: ["s"], shift: false },
  { id: "star", group: "Actions", keys: ["Shift+s"], label: "Star / unstar PR", match: ["s"], shift: true },
  { id: "github", group: "Actions", keys: ["o"], label: "Open in GitHub", match: ["o"] },
  { id: "filters", group: "Actions", keys: ["f"], label: "View options and filters", match: ["f"] },
  { id: "project", group: "Actions", keys: ["p"], label: "Add to project", match: ["p"] },
];

const BY_ID = new Map(SHORTCUTS.map((shortcut) => [shortcut.id, shortcut]));

export function shortcut(id: ShortcutId): Shortcut {
  const found = BY_ID.get(id);
  if (!found) {
    throw new Error(`unknown shortcut ${id}`);
  }
  return found;
}

export function isMac(): boolean {
  return typeof navigator !== "undefined" && /mac|iphone|ipad/i.test(navigator.platform || navigator.userAgent);
}

/** "Mod+K" → "⌘K" on macOS, "Ctrl+K" elsewhere. */
export function displayKey(key: string, mac = isMac()): string {
  return key.startsWith("Mod+") ? (mac ? `⌘${key.slice(4)}` : `Ctrl+${key.slice(4)}`) : key;
}

/** The first key of a shortcut, for tooltips and menu items: "Request review (r)". */
export function hint(id: ShortcutId, mac = isMac()): string {
  return displayKey(shortcut(id).keys[0], mac);
}

/** The shortcut a key press triggers, or null. Shift+x reports "X" in a browser. */
export function matchShortcut(event: KeyEvent): ShortcutId | null {
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  const mod = (event.metaKey || event.ctrlKey) && !event.altKey;
  for (const item of SHORTCUTS) {
    if (!item.match.includes(key)) {
      continue;
    }
    if (item.mod ? !mod : !isPlainShortcut(event)) {
      continue;
    }
    if (item.shift !== undefined && item.shift !== event.shiftKey) {
      continue;
    }
    return item.id;
  }
  return null;
}

export function groupedShortcuts(): { group: ShortcutGroup; items: Shortcut[] }[] {
  return SHORTCUT_GROUPS.map((group) => ({ group, items: SHORTCUTS.filter((item) => item.group === group) }));
}
