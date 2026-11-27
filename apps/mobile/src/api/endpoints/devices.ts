import type { EmberhutHttp } from "../http";
import type { PairingCode, Reply } from "../types";

export function deviceEndpoints(http: EmberhutHttp) {
  return {
    /** This phone's FCM token, so Emberhut reaches it with the app closed. */
    registerPush(token: string): Promise<Reply> {
      return http.post<Reply>("/api/devices", { token, platform: "android" });
    },
    /** A one-time code another device pairs with; only a signed-in device may ask for one. */
    newPairingCode(): Promise<PairingCode> {
      return http.post<PairingCode>("/api/auth/pair/new");
    },
    /** Signs a device out on the server (this one or another of yours). */
    revoke(device: number): Promise<Reply> {
      return http.post<Reply>("/api/settings", { authRevoke: device });
    },
  };
}
