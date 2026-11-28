import { useEffect, useState } from "react";

/** `value` once it has stayed the same for `ms`; an empty one at once (a cleared search shows everything again). */
export function useDebounced(value: string, ms: number): string {
  const [shown, setShown] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setShown(value), value === "" ? 0 : ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return shown;
}
