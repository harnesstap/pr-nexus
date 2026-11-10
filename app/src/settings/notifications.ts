export type NotificationPermission = "granted" | "denied" | "prompt";

const DENIED = "Notifications are off for PR Nexus in system settings.";

/** Grey hint for a denied permission. Granted and not-yet-asked say nothing. */
export function notificationHint(permission: NotificationPermission): string | null {
  switch (permission) {
    case "granted":
    case "prompt":
      return null;
    case "denied":
      return DENIED;
    default: {
      const unexpected: never = permission;
      return unexpected;
    }
  }
}

export function isNotificationPermission(value: string): value is NotificationPermission {
  switch (value) {
    case "granted":
    case "denied":
    case "prompt":
      return true;
    default:
      return false;
  }
}
