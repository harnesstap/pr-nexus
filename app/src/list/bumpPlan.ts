import type { BulkCard } from "../api";

/** What the queue is showing for one destination, so a send can refuse a different one. */
export interface CardSnapshot {
  workspace: string;
  channelId: string;
  channelName: string;
  text: string;
  prIds: number[];
}

export function snapshot(card: BulkCard): CardSnapshot {
  const prIds = card.messages.flatMap((message) => message.prs.map((pr) => pr.pr_id)).sort((left, right) => left - right);
  return {
    workspace: card.target.workspace,
    channelId: card.target.channel_id,
    channelName: card.target.channel_name,
    text: card.messages.map((message) => message.text).join("\n"),
    prIds,
  };
}

function sameSnapshot(left: CardSnapshot, right: CardSnapshot): boolean {
  return (
    left.workspace === right.workspace &&
    left.channelId === right.channelId &&
    left.channelName === right.channelName &&
    left.text === right.text &&
    left.prIds.length === right.prIds.length &&
    left.prIds.every((id, index) => id === right.prIds[index])
  );
}

function key(card: CardSnapshot): string {
  return `${card.workspace}\0${card.channelId}\0${card.prIds.join(",")}\0${card.text}\0${card.channelName}`;
}

/** True when both plans would post the same text to the same channels. */
export function samePlan(shown: BulkCard[], fresh: BulkCard[]): boolean {
  const left = shown.map(snapshot).sort((a, b) => key(a).localeCompare(key(b)));
  const right = fresh.map(snapshot).sort((a, b) => key(a).localeCompare(key(b)));
  return left.length === right.length && left.every((item, index) => sameSnapshot(item, right[index]!));
}

export function sameIds(left: number[], right: number[]): boolean {
  if (left.length !== right.length) {
    return false;
  }
  const a = [...left].sort((x, y) => x - y);
  const b = [...right].sort((x, y) => x - y);
  return a.every((id, index) => id === b[index]);
}

export function quietLabel(hours: number): string {
  if (hours < 48) {
    return `${hours}h`;
  }
  return `${Math.floor(hours / 24)}d`;
}
