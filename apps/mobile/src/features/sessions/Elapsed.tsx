import { useEffect, useState } from "react";
import { Text, type TextProps } from "@/ui";
import { elapsed } from "./sessionList";

export function Elapsed({ since, ...text }: { since: string | null | undefined } & Omit<TextProps, "children">) {
  const start = since ? Date.parse(since) : NaN;
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, []);
  if (Number.isNaN(start)) return null;
  return (
    <Text variant="caption" style={{ fontVariant: ["tabular-nums"] }} {...text}>
      {elapsed(start, now)}
    </Text>
  );
}
