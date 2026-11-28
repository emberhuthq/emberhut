import { describe, expect, it } from "vitest";
import { headerItems } from "./headerButtons";

const sessions = (count: number) => ({
  id: "sessions",
  symbol: "terminal" as const,
  label: `Sessions, ${count} running`,
  onPress: () => undefined,
  count,
  badge: { color: "#111111", background: "#22AA55" },
});
// What UIKit is handed, without the press handler: the snapshot is the header's whole look.
const shown = (count: number) => headerItems([sessions(count)], "#EEEEEE").map(({ onPress: _press, ...item }) => item);

describe("header buttons", () => {
  it("are UIKit bar button items: an SF Symbol in the bar's own size, its label for VoiceOver, no badge at 0", () => {
    expect(shown(0)).toMatchInlineSnapshot(`
      [
        {
          "accessibilityLabel": "Sessions, 0 running",
          "icon": {
            "name": "terminal",
            "type": "sfSymbol",
          },
          "identifier": "sessions",
          "label": "Sessions, 0 running",
          "tintColor": "#EEEEEE",
          "type": "button",
        },
      ]
    `);
  });

  it("carry UIKit's own badge, in the given colours", () => {
    expect(shown(7)[0]).toMatchInlineSnapshot(`
      {
        "accessibilityLabel": "Sessions, 7 running",
        "badge": {
          "style": {
            "backgroundColor": "#22AA55",
            "color": "#111111",
          },
          "value": "7",
        },
        "icon": {
          "name": "terminal",
          "type": "sfSymbol",
        },
        "identifier": "sessions",
        "label": "Sessions, 7 running",
        "tintColor": "#EEEEEE",
        "type": "button",
      }
    `);
  });

  it("count in full up to 99, then 99+", () => {
    expect([9, 10, 12, 99, 100, 120].map((n) => shown(n)[0]?.badge?.value)).toEqual(["9", "10", "12", "99", "99+", "99+"]);
  });
});
