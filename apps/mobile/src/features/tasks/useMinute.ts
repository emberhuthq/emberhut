import { useEffect, useState } from "react";

/**
 * The time now, moved on once a minute, for the plan views' clocks ("quiet 25m", "8m so far"). The state comes back 304
 * while nothing changes, so a quiet task's row and page would not draw again on their own: going quiet is exactly when
 * nothing changes.
 */
export function useMinute(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = setInterval(() => setNow(new Date()), 60_000);
    return () => clearInterval(timer);
  }, []);
  return now;
}
