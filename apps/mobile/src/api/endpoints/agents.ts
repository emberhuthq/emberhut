import type { EmberhutHttp } from "../http";
import type { Agent, AgentsPage } from "../types";

export function agentEndpoints(http: EmberhutHttp) {
  return {
    /** GET /api/agents: every agent, the default agent first. */
    async list(signal?: AbortSignal): Promise<Agent[]> {
      const page = await http.get<AgentsPage>("/api/agents", undefined, { signal });
      return page.agents;
    },
  };
}
