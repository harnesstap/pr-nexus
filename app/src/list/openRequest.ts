/** A tray item or a notification click. Only the two list views are accepted. */
export interface OpenRequest {
  view: "mine" | "to_review";
  prId: number | null;
}

export function parseOpenRequest(payload: unknown): OpenRequest | null {
  if (payload === null || typeof payload !== "object") {
    return null;
  }
  const view = "view" in payload ? payload.view : undefined;
  if (view !== "mine" && view !== "to_review") {
    return null;
  }
  const raw = "pr_id" in payload ? payload.pr_id : undefined;
  const prId = typeof raw === "number" && Number.isFinite(raw) ? raw : null;
  return { view, prId };
}
