import type { EmberhutHttp } from "../http";
import type { ThemeChange, ThemeReply, ThemePage } from "../types";

export function themeEndpoints(http: EmberhutHttp) {
  return {
    /** GET /api/theme: your choice, its tokens and contrast, every preset. */
    get(signal?: AbortSignal): Promise<ThemePage> {
      return http.get<ThemePage>("/api/theme", undefined, { signal });
    },
    /** POST /api/theme: saves the change and answers the page; a bad preset, appearance or hex is a 400. */
    set(change: ThemeChange): Promise<ThemeReply> {
      return http.post<ThemeReply>("/api/theme", change);
    },
    /** The same answer for a custom accent, nothing saved: its contrast while you type. */
    preview(accent: string, signal?: AbortSignal): Promise<ThemeReply> {
      return http.post<ThemeReply>("/api/theme", { accent, preview: true }, { signal });
    },
  };
}
