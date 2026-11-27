import { describe, expect, it } from "vitest";
import { daysFromToday, deadlineText, parseDay, parseInstant, relative, span } from "./time";

// Local times, so the checks hold in any time zone.
const at = (day: number, hour: number, minute = 0) => new Date(2026, 10, day, hour, minute);
const now = at(24, 13, 30);

describe("daysFromToday", () => {
  it("counts calendar days, whatever the time of day", () => {
    expect(daysFromToday(at(23, 13), now)).toBe(-1);
    expect(daysFromToday(at(23, 23, 59), now)).toBe(-1);
    expect(daysFromToday(at(24, 0, 1), now)).toBe(0);
    expect(daysFromToday(at(24, 23, 59), now)).toBe(0);
    expect(daysFromToday(at(25, 23), now)).toBe(1);
    expect(daysFromToday(at(20, 1), now)).toBe(-4);
  });

  it("holds across a month's end", () => {
    expect(daysFromToday(new Date(2026, 11, 1, 9), now)).toBe(7);
  });
});

describe("relative", () => {
  it("says minutes and hours within a day", () => {
    expect(relative(new Date(now.getTime() - 20_000), now)).toBe("just now");
    expect(relative(at(24, 13, 25), now)).toBe("5 min ago");
    expect(relative(at(24, 10, 30), now)).toBe("3 h ago");
    expect(relative(at(24, 13, 50), now)).toBe("in 20 min");
  });

  it("names yesterday and tomorrow by the calendar", () => {
    expect(relative(at(23, 13), now)).toBe("yesterday");
    expect(relative(at(22, 23), now)).toBe("2 days ago");
    expect(relative(at(25, 23), now)).toBe("tomorrow");
    expect(relative(at(26, 14), now)).toBe("in 2 days");
  });

  it("gives a date a week or more away", () => {
    expect(relative(at(12, 9), now)).toBe("12 Nov");
  });
});

describe("deadlineText", () => {
  it("reads late, today, tomorrow, a weekday, then a date", () => {
    expect(deadlineText(at(21, 0), now)).toBe("3d late");
    expect(deadlineText(at(24, 0), now)).toBe("Today");
    expect(deadlineText(at(25, 0), now)).toBe("Tomorrow");
    expect(deadlineText(at(27, 0), now)).toBe("Fri");
    expect(deadlineText(new Date(2026, 11, 3), now)).toBe("3 Dec");
  });
});

describe("parsing", () => {
  it("reads a day as local midnight, and an instant's date part", () => {
    expect(parseDay("2026-11-24")).toEqual(at(24, 0));
    expect(parseDay("2026-11-24T22:00:00Z")).toEqual(at(24, 0));
    expect(parseDay("soon")).toBeNull();
    expect(parseDay(null)).toBeNull();
  });

  it("reads instants and refuses what isn't one", () => {
    expect(parseInstant("2026-11-24T12:00:00Z")?.getTime()).toBe(Date.UTC(2026, 10, 24, 12));
    expect(parseInstant("later")).toBeNull();
    expect(parseInstant(undefined)).toBeNull();
  });
});

describe("span", () => {
  it("reads seconds, minutes, then hours and minutes", () => {
    expect(span(at(24, 13), new Date(at(24, 13).getTime() + 40_000))).toBe("40 s");
    expect(span(at(24, 13), at(24, 13, 12))).toBe("12 min");
    expect(span(at(24, 13), at(24, 15))).toBe("2 h");
    expect(span(at(24, 13), at(24, 15, 5))).toBe("2 h 5 min");
  });
});
