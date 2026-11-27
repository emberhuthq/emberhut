import type { EmberhutHttp } from "../http";
import type { AgentProvider, ProviderRequest, ProviderTest } from "../types";

/** The server checks a provider within 15 s: twice that leaves room for the way there and back. */
const checkMs = 30_000;

export function agentProviderEndpoints(http: EmberhutHttp) {
  return {
    /** GET /api/agent/provider: what is saved, without the key. */
    provider(signal?: AbortSignal): Promise<AgentProvider> {
      return http.get<AgentProvider>("/api/agent/provider", undefined, { signal });
    },
    /** PUT /api/agent/provider: checked with the provider first, saved only when it passes. */
    save(request: ProviderRequest): Promise<AgentProvider> {
      return http.put<AgentProvider>("/api/agent/provider", request, { timeoutMs: checkMs });
    },
    /** DELETE /api/agent/provider: forgets what is saved and every key. */
    forget(): Promise<AgentProvider> {
      return http.delete<AgentProvider>("/api/agent/provider");
    },
    /** POST /api/agent/provider/test: checked with the provider, nothing saved; answers the models it lists within `timeoutMs`. */
    test(request: ProviderRequest, timeoutMs = checkMs): Promise<ProviderTest> {
      return http.post<ProviderTest>("/api/agent/provider/test", request, { timeoutMs });
    },
  };
}
