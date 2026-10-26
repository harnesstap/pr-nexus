import { expect, test } from "vitest";
import { disableNativeContextMenu } from "./contextMenu";

function rightClick(): boolean {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  document.body.dispatchEvent(event);
  return event.defaultPrevented;
}

test("release builds suppress the native context menu, dev builds keep it", () => {
  const dev = disableNativeContextMenu(document, false);
  expect(rightClick()).toBe(false);
  dev();
  const release = disableNativeContextMenu(document, true);
  expect(rightClick()).toBe(true);
  release();
  expect(rightClick()).toBe(false);
});
