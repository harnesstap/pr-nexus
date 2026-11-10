import { expect, test } from "vitest";

import { isNotificationPermission, notificationHint } from "./notifications";

test("a denied notification permission is a plain hint", () => {
  expect(notificationHint("denied")).toBe("Notifications are off for PR Nexus in system settings.");
  expect(notificationHint("granted")).toBeNull();
  expect(notificationHint("prompt")).toBeNull();
  expect(isNotificationPermission("denied")).toBe(true);
  expect(isNotificationPermission("later")).toBe(false);
});
