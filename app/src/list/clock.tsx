import { createContext, useContext, type ReactNode } from "react";
import { ago } from "./model";

const ClockContext = createContext<Date | null>(null);

/** Shares one list clock so a row can skip rendering while its times update. */
export function ClockProvider({ now, children }: { now: Date; children: ReactNode }) {
  return <ClockContext.Provider value={now}>{children}</ClockContext.Provider>;
}

/**
 * The list clock. Outside a provider this is the time of the render and does
 * not subscribe to later ticks.
 */
export function useNow(): Date {
  return useContext(ClockContext) ?? new Date();
}

/** "2h ago" for one timestamp. Subscribes to the shared clock on its own. */
export function RelativeTime({ iso }: { iso: string }) {
  const now = useNow();
  return <>{ago(iso, now)}</>;
}
