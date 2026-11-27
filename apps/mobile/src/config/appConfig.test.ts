import { describe, expect, it } from "vitest";
import appConfig from "../../app.config";
import { cidrOf, privateIpv4Ranges, privateIpv6Ranges } from "@/api/localNetwork";
import { dialect } from "@/api/dialect";
import { PUSH_CHANNEL } from "@/api/wire";
import { channelId } from "@/push/link";

const config = appConfig({ config: {} } as Parameters<typeof appConfig>[0]);

describe("app.config", () => {
  it("lets plain http reach only the private ranges on iOS, never every host", () => {
    const ats = config.ios?.infoPlist?.NSAppTransportSecurity as Record<string, unknown>;
    expect(ats).not.toHaveProperty("NSAllowsArbitraryLoads");
    expect(ats).not.toHaveProperty("NSAllowsLocalNetworking");
    const exceptions = ats.NSExceptionDomains as Record<string, unknown>;
    expect(Object.keys(exceptions).sort()).toEqual([...privateIpv4Ranges.map(cidrOf), ...privateIpv6Ranges].sort());
    for (const exception of Object.values(exceptions)) expect(exception).toEqual({ NSExceptionAllowsInsecureHTTPLoads: true });
  });

  it("names the app from its domain", () => {
    expect(config.ios?.bundleIdentifier).toBe("com.emberhut.mobile");
    expect(config.android?.package).toBe("com.emberhut.mobile");
    expect(config.scheme).toBe("emberhut");
  });

  it("makes the push channel the server names, the one the app creates", () => {
    const notifications = config.plugins?.find((p) => Array.isArray(p) && p[0] === "expo-notifications") as [
      string,
      { defaultChannel: string },
    ];
    expect(notifications[1].defaultChannel).toBe(dialect.writeText(PUSH_CHANNEL));
    expect(channelId).toBe(notifications[1].defaultChannel);
  });
});
