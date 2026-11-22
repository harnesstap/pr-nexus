/**
 * How long ago a review was requested, and whether that wait is on time.
 * The painted age is wall-clock time. Due and late follow the review SLA,
 * including weekday hours when that setting is on.
 */
import type { ReviewWait } from "../api";
import { reviewRemainingMs, reviewUrgency, type ReviewSla, type ReviewUrgency } from "./sla";

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;

/** Two units at most. A zero remainder is dropped. Under a second is `0s`. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / SECOND));
  const parts: [number, string][] = [
    [Math.floor(total / (WEEK / SECOND)), "w"],
    [Math.floor((total % (WEEK / SECOND)) / (DAY / SECOND)), "d"],
    [Math.floor((total % (DAY / SECOND)) / (HOUR / SECOND)), "h"],
    [Math.floor((total % (HOUR / SECOND)) / (MINUTE / SECOND)), "m"],
    [total % (MINUTE / SECOND), "s"],
  ];
  const start = parts.findIndex(([count]) => count > 0);
  if (start < 0) {
    return "0s";
  }
  const first = parts[start];
  const second = parts[start + 1];
  if (!first) {
    return "0s";
  }
  if (!second || second[0] === 0) {
    return `${first[0]}${first[1]}`;
  }
  return `${first[0]}${first[1]} ${second[0]}${second[1]}`;
}

/** Local weekday and time, for example `Mon 14:02`. */
export function formatRequestWhen(at: Date): string {
  const day = WEEKDAYS[at.getDay()] ?? "";
  const hours = String(at.getHours()).padStart(2, "0");
  const minutes = String(at.getMinutes()).padStart(2, "0");
  return `${day} ${hours}:${minutes}`;
}

export interface ReviewWaitText {
  short: string;
  label: string;
  detail?: string;
  urgency: ReviewUrgency;
}

/** The chip, its tooltip, and the colour. */
export function reviewWaitText(wait: ReviewWait, now: Date, sla: ReviewSla, warnHours: number): ReviewWaitText {
  const at = new Date(wait.at);
  const since = at.getTime();
  const age = Math.max(0, now.getTime() - since);
  const urgency = reviewUrgency(since, now, sla, warnHours);
  const remaining = reviewRemainingMs(since, now, sla);
  const tail = dueTail(urgency, remaining);
  const label = `Review requested ${formatElapsed(age)} ago (${formatRequestWhen(at)}) · ${tail}`;
  const notes = [approximateNote(wait), sla.workingDays ? "Due and late count weekday hours." : ""].filter((note) => note.length > 0);
  return {
    short: formatElapsed(age),
    label,
    detail: notes.length > 0 ? notes.join(" ") : undefined,
    urgency,
  };
}

function dueTail(urgency: ReviewUrgency, remainingMs: number): string {
  switch (urgency) {
    case "late":
      return `late by ${formatElapsed(-remainingMs)}`;
    case "on_time":
    case "approaching":
      return remainingMs <= 0 ? "due now" : `due in ${formatElapsed(remainingMs)}`;
    default: {
      const unreachable: never = urgency;
      return unreachable;
    }
  }
}

function approximateNote(wait: ReviewWait): string {
  if (wait.exact) {
    return "";
  }
  switch (wait.basis) {
    case "request":
      return "Approximate: the review request time was not in the latest timeline slice.";
    case "ready":
      return "Approximate: GitHub did not return the review request time, so this uses when it was marked ready for review.";
    case "opened":
      return "Approximate: GitHub did not return the review request time, so this uses when the pull request was opened.";
    default: {
      const unreachable: never = wait.basis;
      return unreachable;
    }
  }
}
