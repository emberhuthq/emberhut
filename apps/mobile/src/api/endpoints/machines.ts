import type { EmberhutHttp } from "../http";
import type { MachinesPage } from "../types";

export function machineEndpoints(http: EmberhutHttp) {
  return {
    /**
     * GET /api/machines: each runner machine's latest sample; a server on older code answers 404. Without `history`
     * (`?history=0`) the server leaves each machine's day of 5-minute points out, and they come back empty here.
     */
    async list(signal?: AbortSignal, { history = true }: { history?: boolean } = {}): Promise<MachinesPage> {
      const page = await http.get<MachinesPage>("/api/machines", history ? undefined : { history: 0 }, { signal });
      return history ? page : { ...page, machines: page.machines.map((m) => ({ ...m, history: m.history ?? [] })) };
    },
  };
}
