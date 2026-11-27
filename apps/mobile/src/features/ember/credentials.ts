import type { AgentProvider, ProviderKind, ProviderModel, ProviderRequest } from "@/api";

/** A box a provider's row opens: its API key, or where its API is. */
export interface ProviderField {
  name: "key" | "baseUrl";
  /** Needed for a test; otherwise a blank one is left out and the server uses its own default. */
  needed: boolean;
  /** The base URL's example; the key's comes from keyPlaceholder. */
  placeholder?: string;
}

/** One provider the picker offers. */
export interface ProviderRow {
  kind: ProviderKind;
  name: string;
  /** One line on what it is. */
  line: string;
  /** More, shown once the row is open. */
  hint?: string;
  tag?: string;
  fields: readonly ProviderField[];
}

const key: ProviderField = { name: "key", needed: true };

/** The providers offered, in this order. The model is never typed: a passed test lists the provider's models to pick from. */
export const ROWS: readonly ProviderRow[] = [
  {
    kind: "anthropic",
    name: "Claude",
    tag: "Recommended",
    line: "Best with tools. Uses your Anthropic API key, pay as you go.",
    hint: "Get a key at console.anthropic.com · signing in with a Claude Pro or Max plan comes later",
    fields: [key],
  },
  { kind: "openai", name: "OpenAI", line: "GPT models, with your OpenAI key.", fields: [key] },
  { kind: "deepseek", name: "DeepSeek", line: "Low cost and strong at code, with your DeepSeek key.", fields: [key] },
  { kind: "openrouter", name: "OpenRouter", line: "One key, hundreds of models from many makers.", fields: [key] },
  {
    kind: "ollama",
    name: "Ollama",
    line: "Runs on the computer your ember lives on. Free and private, but slower.",
    hint: "Nothing leaves the computer your ember lives on. Smaller models handle tools less well.",
    // The placeholder is the server's own default: a blank box is left out, so the server uses it.
    fields: [{ name: "baseUrl", needed: false, placeholder: "http://localhost:11434" }],
  },
  {
    kind: "openai-compatible",
    name: "Another service",
    line: "Any OpenAI-style service: its address, a key and a model.",
    fields: [
      { name: "baseUrl", needed: true, placeholder: "https://…/v1" },
      { name: "key", needed: false },
    ],
  },
];

/** The tag of a row the agent can't run on yet. */
export const COMING_NEXT = "Coming next";

/** Providers the agent runs on today, by the server's documented rule: all but openai and openai-compatible. */
const runsToday: ReadonlySet<string> = new Set(["claude", "anthropic", "deepseek", "openrouter", "ollama", "anthropic-compatible"]);

/**
 * Whether the agent can think with `kind`, so whether its row can be picked. The server says `usable` only for the saved
 * provider, so that is its word when `kind` is the saved one; for any other the server's documented rule stands in: only
 * openai and openai-compatible need a bridge that isn't built yet, and a provider this app doesn't know can't be picked.
 */
export function canRunOn(kind: ProviderKind, saved: AgentProvider | undefined): boolean {
  if (saved?.provider === kind) return saved.usable;
  return runsToday.has(kind);
}

export const rowOf = (kind: ProviderKind | undefined): ProviderRow | undefined => ROWS.find((r) => r.kind === kind);

/** A row's tag: "Coming next" while the agent can't run on it, else its own. */
export const rowTag = (row: ProviderRow, saved: AgentProvider | undefined): string | undefined =>
  canRunOn(row.kind, saved) ? row.tag : COMING_NEXT;

/** The key box's placeholder: the saved key's last four when this kind's key is saved. */
export function keyPlaceholder(kind: ProviderKind, saved: AgentProvider | undefined): string {
  return saved?.provider === kind && saved.keySet && saved.keyHint ? `Saved key ending in ${saved.keyHint}` : "API key";
}

/** "Forget this key" shows on the saved provider's row while a key is saved for it. */
export const canForget = (kind: ProviderKind, saved: AgentProvider | undefined): boolean => saved?.provider === kind && saved.keySet;

/** A provider's name for people, also for the saved ones without a row of their own. */
export function providerName(kind: ProviderKind): string {
  if (kind === "claude") return "Claude, on its own sign-in";
  if (kind === "anthropic-compatible") return "An Anthropic-style service";
  return rowOf(kind)?.name ?? kind;
}

/** What is saved, in one line: the provider, its model, its key's last four. */
export function savedLine(saved: AgentProvider | undefined): string {
  if (!saved?.provider) return "Not chosen yet";
  const hint = saved.keySet && saved.keyHint ? `key ending ${saved.keyHint}` : undefined;
  return [providerName(saved.provider), saved.model, hint].filter(Boolean).join(" · ");
}

/** Why what is saved isn't what the agent thinks with now, when it isn't. */
export function savedStatus(saved: AgentProvider | undefined): string | undefined {
  if (!saved?.provider) return undefined;
  if (!saved.usable) return "Saved. Your ember can't think with it yet.";
  if (!saved.inUse) return "Saved, not in use: this server's own accounts come first.";
  return undefined;
}

export const modelLabel = (model: ProviderModel): string => model.name ?? model.id;

/** A list longer than this gets a box to find a model in. */
export const MODELS_FINDABLE = 8;

/** The models whose id or name has what was typed, any case. */
export function modelsShown(models: readonly ProviderModel[], query: string): ProviderModel[] {
  const q = query.trim().toLowerCase();
  if (q === "") return [...models];
  return models.filter((m) => m.id.toLowerCase().includes(q) || (m.name?.toLowerCase().includes(q) ?? false));
}

/** The words of a passed test: how many models the provider lists. */
export function passedLine(models: readonly ProviderModel[]): string {
  if (models.length === 0) return "It works";
  return `It works · ${models.length} ${models.length === 1 ? "model" : "models"}`;
}

/** The title of a failed test; the server's sentence goes under it. */
export const FAILED_TITLE = "Didn't answer";

/** A failed request's sentence as the server put it (an ApiError's message), or the API client for a server out of reach. */
export function failureReason(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Something went wrong";
}

export type TestState =
  | { step: "idle" }
  | { step: "testing"; version: number }
  | {
      step: "passed";
      version: number;
      models: ProviderModel[];
      /** The provider's answer to a test turn, when one was asked; nothing asks for one yet. */
      answer?: string;
    }
  | { step: "failed"; version: number; reason: string };

/** The picker: the row open, what is typed in it, its test, the model chosen, a save and a forget. */
export interface PickerState {
  selected?: ProviderKind;
  /** The key typed now: the server gives no saved key back, so every test and save sends this one. */
  key: string;
  baseUrl: string;
  test: TestState;
  /** The model picked from a passed test's list. */
  model?: string;
  saving: boolean;
  saveError?: string;
  forget: "idle" | "confirming" | "forgetting";
  forgetError?: string;
  /** Goes up with every change to what would be sent: a test's result counts only for the values it was of. */
  version: number;
}

export type PickerAction =
  | { type: "select"; kind: ProviderKind; saved: AgentProvider | undefined }
  | { type: "edit"; field: ProviderField["name"]; value: string }
  | { type: "testStarted" }
  | { type: "testPassed"; version: number; models: ProviderModel[]; saved: AgentProvider | undefined }
  | { type: "testFailed"; version: number; reason: string }
  | { type: "chooseModel"; id: string }
  | { type: "saveStarted" }
  | { type: "saveFailed"; reason: string }
  | { type: "saved" }
  | { type: "askForget" }
  | { type: "cancelForget" }
  | { type: "forgetStarted" }
  | { type: "forgetFailed"; reason: string }
  | { type: "forgotten" };

const idle: TestState = { step: "idle" };

/** The picker as it opens: the saved provider's row open when it has one, its saved base URL in its box. */
export function initialPicker(saved: AgentProvider | undefined): PickerState {
  const row = rowOf(saved?.provider);
  const open = row && canRunOn(row.kind, saved) ? row : undefined;
  return {
    selected: open?.kind,
    key: "",
    baseUrl: open && hasField(open, "baseUrl") ? (saved?.baseUrl ?? "") : "",
    test: idle,
    saving: false,
    forget: "idle",
    version: 0,
  };
}

const hasField = (row: ProviderRow, name: ProviderField["name"]): boolean => row.fields.some((f) => f.name === name);

/** The model to start on: the saved one when it is this provider's and listed, else the first listed. */
export function preselectModel(kind: ProviderKind, models: readonly ProviderModel[], saved: AgentProvider | undefined): string | undefined {
  if (saved?.provider === kind && saved.model !== undefined && models.some((m) => m.id === saved.model)) return saved.model;
  return models[0]?.id;
}

export function pickerReducer(state: PickerState, action: PickerAction): PickerState {
  switch (action.type) {
    case "select": {
      if (action.kind === state.selected || !canRunOn(action.kind, action.saved)) return state;
      const row = rowOf(action.kind);
      const savedUrl = action.saved?.provider === action.kind && row && hasField(row, "baseUrl") ? action.saved.baseUrl : undefined;
      return {
        ...state,
        selected: action.kind,
        key: "",
        baseUrl: savedUrl ?? "",
        test: idle,
        model: undefined,
        saveError: undefined,
        forgetError: undefined,
        version: state.version + 1,
      };
    }
    case "edit": {
      if (state.saving) return state;
      const typed = action.field === "key" ? { key: action.value } : { baseUrl: action.value };
      // What was tested isn't what is typed any more.
      return { ...state, ...typed, test: idle, model: undefined, saveError: undefined, version: state.version + 1 };
    }
    case "testStarted":
      return canTest(state)
        ? { ...state, test: { step: "testing", version: state.version }, model: undefined, saveError: undefined }
        : state;
    case "testPassed":
      if (!current(state, action.version) || state.selected === undefined) return state;
      return {
        ...state,
        test: { step: "passed", version: action.version, models: action.models },
        model: preselectModel(state.selected, action.models, action.saved),
      };
    case "testFailed":
      if (!current(state, action.version)) return state;
      return { ...state, test: { step: "failed", version: action.version, reason: action.reason } };
    case "chooseModel":
      if (state.test.step !== "passed" || !state.test.models.some((m) => m.id === action.id)) return state;
      return { ...state, model: action.id };
    case "saveStarted":
      return canSave(state) ? { ...state, saving: true, saveError: undefined } : state;
    case "saveFailed":
      return { ...state, saving: false, saveError: action.reason };
    case "saved":
      // The key goes to the server and nowhere else: none is kept here once it's saved.
      return { ...state, key: "", saving: false, saveError: undefined, test: idle, version: state.version + 1 };
    case "askForget":
      return state.forget === "idle" ? { ...state, forget: "confirming", forgetError: undefined } : state;
    case "cancelForget":
      return state.forget === "confirming" ? { ...state, forget: "idle" } : state;
    case "forgetStarted":
      return state.forget === "confirming" ? { ...state, forget: "forgetting" } : state;
    case "forgetFailed":
      return { ...state, forget: "idle", forgetError: action.reason };
    case "forgotten":
      return { ...state, forget: "idle", forgetError: undefined };
  }
}

/** A test's result is for the values typed now, and still awaited. */
const current = (state: PickerState, version: number): boolean => state.test.step === "testing" && state.test.version === version;

/**
 * What a test or a save sends: trimmed, a blank optional box left out (never ""), a key only to a provider that takes
 * one, and the model only on a save. Undefined while no row is open.
 */
export function request(state: PickerState, purpose: "test" | "save"): ProviderRequest | undefined {
  const row = rowOf(state.selected);
  if (!row) return undefined;
  const typed = (name: ProviderField["name"], value: string) => (hasField(row, name) && value.trim() !== "" ? value.trim() : undefined);
  const apiKey = typed("key", state.key);
  const baseUrl = typed("baseUrl", state.baseUrl);
  const model = purpose === "save" ? state.model : undefined;
  return {
    provider: row.kind,
    ...(apiKey !== undefined && { apiKey }),
    ...(baseUrl !== undefined && { baseUrl }),
    ...(model !== undefined && { model }),
  };
}

/** "Test it" can go: every needed box is filled, and nothing is under way. */
export function canTest(state: PickerState): boolean {
  const row = rowOf(state.selected);
  if (!row || state.saving || state.test.step === "testing") return false;
  const filled = (name: ProviderField["name"]) => (name === "key" ? state.key : state.baseUrl).trim() !== "";
  return row.fields.every((f) => !f.needed || filled(f.name));
}

/** "Use this" can go: the test passed for exactly what is typed, and a model is chosen unless the provider lists none. */
export function canSave(state: PickerState): boolean {
  const { test } = state;
  if (state.saving || test.step !== "passed" || test.version !== state.version) return false;
  return state.model !== undefined || test.models.length === 0;
}
