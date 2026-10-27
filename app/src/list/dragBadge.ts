/** Label on the drag image: what a drop onto a project will add. */
export function dragBadgeLabel(count: number): string {
  return count === 1 ? "1 PR" : `${count} PRs`;
}

/**
 * Replaces the browser's row snapshot with an "N PRs" pill so a drag of the
 * whole selection shows how many PRs it carries. The element only has to exist
 * while the browser renders the drag image, so it is removed on the next tick.
 */
export function setDragBadge(dataTransfer: DataTransfer, count: number): void {
  if (typeof dataTransfer.setDragImage !== "function") {
    return;
  }
  const badge = document.createElement("div");
  badge.textContent = dragBadgeLabel(count);
  badge.dataset.dragBadge = "true";
  badge.className = "pointer-events-none fixed -left-[1000px] top-0 rounded-full bg-violet-500 px-3 py-1 text-xs font-semibold text-white shadow-lg";
  document.body.append(badge);
  dataTransfer.setDragImage(badge, -12, -12);
  window.setTimeout(() => badge.remove(), 0);
}
