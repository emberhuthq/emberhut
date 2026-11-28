import type { EmberhutHttp } from "../http";
import type { BrowserInfo, BrowserInput, BrowserSession } from "../types";

export function browserEndpoints(http: EmberhutHttp) {
  return {
    /** The live view's address and credentials: view-only, or control once this device took over. */
    session(device: string): Promise<BrowserSession> {
      // Its user and password are the stream's own: a word of the build's dialect among them would be rewritten.
      return http.get<BrowserSession>("/api/browser/session", { device }, { raw: true });
    },
    /** Take over, hand back, and the toolbar (back, go, tabs). */
    input(input: BrowserInput): Promise<BrowserInfo> {
      return http.post<BrowserInfo>("/api/browser/input", input);
    },
    /** A file for the page's upload field, when the page asked for one (state.browser.fileRequest). */
    upload(file: string, name: string): Promise<BrowserInfo> {
      return http.json<BrowserInfo>("/api/browser/upload", { query: { name }, file, timeoutMs: 120_000 });
    },
    /** The latest frame while Emberhut drives (refreshed at most every 5 s); `t` keeps caches out of it. */
    frameSource(t: number) {
      return http.fileSource(`/api/browser/frame.jpg?t=${t}`);
    },
  };
}
