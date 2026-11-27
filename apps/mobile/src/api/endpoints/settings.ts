import type { EmberhutHttp } from "../http";
import type {
  AccountUseResult,
  EnvironmentsPage,
  FactsPage,
  EmberhutState,
  Reply,
  SettingsChange,
  SwitchAction,
  SwitchPage,
} from "../types";

export function settingsEndpoints(http: EmberhutHttp) {
  return {
    /** POST /api/settings: answers the whole state of `change.room` (or Main). */
    apply(change: SettingsChange): Promise<Reply & EmberhutState> {
      return http.post<Reply & EmberhutState>("/api/settings", change, { timeoutMs: 30_000 });
    },
    facts(): Promise<FactsPage> {
      return http.get<FactsPage>("/api/facts");
    },
    switches(): Promise<SwitchPage> {
      return http.get<SwitchPage>("/api/switches");
    },
    setSwitch(action: SwitchAction): Promise<SwitchPage> {
      return http.post<SwitchPage>("/api/switches", action, { timeoutMs: 30_000 });
    },
    environments(): Promise<EnvironmentsPage> {
      return http.get<EnvironmentsPage>("/api/environments", undefined, { timeoutMs: 30_000 });
    },
    checkEnvironment(id: string, kind: string): Promise<EnvironmentsPage> {
      return http.post<EnvironmentsPage>("/api/environments/check", { id, kind }, { timeoutMs: 60_000 });
    },
    /** Pins Emberhut to an account, or "auto" for the one with the most room. */
    useAccount(account: string): Promise<AccountUseResult> {
      return http.post<AccountUseResult>("/api/accounts/use", { account });
    },
    /** Every account's use checked now (at most once in 2 minutes on the server); the state brings the numbers. */
    checkUsage(): Promise<Reply> {
      return http.post<Reply>("/api/usage/check");
    },
  };
}
