import { describe, expect, it } from "vitest";
import { type, typeFor } from "./tokens";

describe("type scale", () => {
  it("sets body at iOS's 17 on iOS and Material's 16 on Android, the rest alike", () => {
    expect(typeFor("ios").body.fontSize).toBe(17);
    expect(typeFor("android").body.fontSize).toBe(16);
    expect(typeFor("android").body.lineHeight).toBe(type.body.lineHeight);
    expect(typeFor("android").headline).toEqual(type.headline);
  });
});
