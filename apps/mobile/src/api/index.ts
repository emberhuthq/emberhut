import { agentProviderEndpoints } from "./endpoints/agentProvider";
import { agentEndpoints } from "./endpoints/agents";
import { approvalEndpoints } from "./endpoints/approvals";
import { authEndpoints } from "./endpoints/auth";
import { browserEndpoints } from "./endpoints/browser";
import { chatEndpoints } from "./endpoints/chat";
import { deviceEndpoints } from "./endpoints/devices";
import { fileEndpoints } from "./endpoints/files";
import { machineEndpoints } from "./endpoints/machines";
import { partyEndpoints } from "./endpoints/parties";
import { sessionEndpoints } from "./endpoints/sessions";
import { settingsEndpoints } from "./endpoints/settings";
import { stateEndpoints } from "./endpoints/state";
import { taskEndpoints } from "./endpoints/tasks";
import { themeEndpoints } from "./endpoints/theme";
import type { EmberhutHttp } from "./http";

/** Every endpoint the app uses, grouped as the server's routes are. */
export function createApi(http: EmberhutHttp) {
  return {
    http,
    auth: authEndpoints(http),
    state: stateEndpoints(http),
    chat: chatEndpoints(http),
    files: fileEndpoints(http),
    tasks: taskEndpoints(http),
    settings: settingsEndpoints(http),
    sessions: sessionEndpoints(http),
    browser: browserEndpoints(http),
    devices: deviceEndpoints(http),
    machines: machineEndpoints(http),
    parties: partyEndpoints(http),
    theme: themeEndpoints(http),
    agents: agentEndpoints(http),
    agentProvider: agentProviderEndpoints(http),
    approvals: approvalEndpoints(http),
  };
}

export type EmberhutApi = ReturnType<typeof createApi>;

export { agentEvent, agentStep } from "./agentSteps";
export {
  AuthSession,
  type AuthEvent,
  type KeptMachine,
  type KeptSession,
  type PairOptions,
  type PairOutcome,
  type SecureStorage,
} from "./auth";
export {
  checkServerAddress,
  createServerAddress,
  serverUrlFrom,
  slotPrefix,
  type AddressCheck,
  type ServerAddress,
  type Slot,
} from "./server";
export { createHealthCheck, healthTimeoutMs, type HealthCheck } from "./health";
export { ApiError, StorageError } from "./errors";
export { isServerPath, type Fetch, type UploadFile } from "./fetch";
export {
  contractWords,
  createDialect,
  dialect,
  fromServer,
  hasWireDialect,
  identityDialect,
  jsonFromServer,
  jsonToServer,
  parseDialect,
  toServer,
  withTaskKeyPrefix,
  type Dialect,
  type DialectTable,
} from "./dialect";
export { EmberhutHttp, type RequestAuth } from "./http";
export { createServerHttp, type ServerHttpOptions, type ServerPlace } from "./places";
export { createReach, firstReachable, type Reach, type RouteAnswer } from "./reach";
export {
  ASSISTANT,
  EXTERNAL_THREADS_GROUP,
  LINK_SCHEME,
  LOOKING_HEADER,
  OWNER,
  PLAN_HISTORY_OPENINGS,
  PUSH_CHANNEL,
  SESSION_TASK_ROUTE,
  STEP_STATE,
  TASK_KEY_PATTERN,
  CONTRACT_TASK_KEY_PREFIX,
  TASK_KEY_PREFIX,
  taskKeyPrefixFrom,
  taskKey,
  type Author,
  type StepState,
} from "./wire";
export { APPROVALS_EVENT, followParty, followRoomLive, followSession, type RoomLiveOptions } from "./live";
export type * from "./types";
