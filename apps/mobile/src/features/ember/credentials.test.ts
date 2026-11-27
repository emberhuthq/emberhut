import { describe, expect, it } from "vitest";
import { ApiError, type AgentProvider, type ProviderModel } from "@/api";
import {
  COMING_NEXT,
  FAILED_TITLE,
  FOUND_ON,
  OLLAMA_LOOK,
  ROWS,
  canForget,
  canPickRow,
  canRunOn,
  canSave,
  canTest,
  comingNextFor,
  failureReason,
  fieldsOf,
  initialPicker,
  keyPlaceholder,
  modelLabel,
  modelsShown,
  passedLine,
  pickerReducer,
  preselectModel,
  providerName,
  request,
  rowKinds,
  rowOf,
  rowTag,
  savedLine,
  savedStatus,
  type PickerAction,
  type PickerState,
} from "./credentials";

const exampleKey = "sk-ant-example-key-0000";
const models: ProviderModel[] = [{ id: "example-model-a" }, { id: "example-model-b", name: "Example Model B" }];

function saved(fields: Partial<AgentProvider> = {}): AgentProvider {
  return { provider: "anthropic", keySet: true, keyHint: "0000", usable: true, inUse: true, ...fields };
}

const nothing: AgentProvider = { keySet: false, usable: false, inUse: false };

function run(actions: PickerAction[], from: PickerState = initialPicker(undefined)): PickerState {
  return actions.reduce(pickerReducer, from);
}

/** The picker on `kind`, its test passed with `listed`. */
function passed(kind: string, typed: PickerAction[], listed = models, on?: AgentProvider): PickerState {
  const before = run([{ type: "select", kind, saved: on }, ...typed, { type: "testStarted" }]);
  return pickerReducer(before, { type: "testPassed", version: before.version, models: listed, saved: on });
}

describe("the provider rows", () => {
  it("offers six providers in order, with their fields", () => {
    expect(ROWS.map((r) => [r.kind, r.name, r.tag ?? null])).toEqual([
      ["anthropic", "Claude", "Recommended"],
      ["openai", "OpenAI", null],
      ["deepseek", "DeepSeek", null],
      ["openrouter", "OpenRouter", null],
      ["ollama", "Ollama", null],
      ["openai-compatible", "Another service", null],
    ]);
    const kinds = ROWS.flatMap(rowKinds);
    const fields = Object.fromEntries(kinds.map((kind) => [kind, fieldsOf(kind).map((f) => `${f.name}${f.needed ? "!" : "?"}`)]));
    expect(fields).toEqual({
      anthropic: ["key!"],
      openai: ["key!"],
      deepseek: ["key!"],
      openrouter: ["key!"],
      ollama: ["baseUrl?"],
      "openai-compatible": ["baseUrl!", "key?"],
      "anthropic-compatible": ["baseUrl!", "key!"],
    });
    expect(fieldsOf("ollama")[0]?.placeholder).toBe("http://localhost:11434");
    expect(ROWS[0]?.hint).toContain("console.anthropic.com");
  });

  it("takes the server's usable for the saved provider and its documented rule for the rest", () => {
    expect(ROWS.filter((r) => !canRunOn(r.kind, undefined)).map((r) => r.kind)).toEqual(["openai", "openai-compatible"]);
    expect(canRunOn("openai", saved({ provider: "openai", usable: true }))).toBe(true);
    expect(canRunOn("anthropic", saved({ usable: false }))).toBe(false);
    expect(canRunOn("deepseek", saved({ usable: false }))).toBe(true);
    expect(canRunOn("claude", nothing)).toBe(true);
    expect(canRunOn("some-new-provider", undefined)).toBe(false);
    expect(canRunOn("some-new-provider", saved({ provider: "some-new-provider", usable: true }))).toBe(true);
  });

  it("tags a row Coming next only while it can't be run on", () => {
    const openai = ROWS.find((r) => r.kind === "openai")!;
    expect(rowTag(openai, undefined)).toBe(COMING_NEXT);
    expect(rowTag(openai, saved({ provider: "openai", usable: true }))).toBeUndefined();
    expect(rowTag(ROWS[0]!, undefined)).toBe("Recommended");
  });

  it("names the saved key in the key box and offers to forget it on its own row only", () => {
    expect(keyPlaceholder("anthropic", saved())).toBe("Saved key ending in 0000");
    expect(keyPlaceholder("deepseek", saved())).toBe("API key");
    expect(keyPlaceholder("anthropic", saved({ keySet: false, keyHint: undefined }))).toBe("API key");
    expect(keyPlaceholder("anthropic", undefined)).toBe("API key");
    expect(canForget("anthropic", saved())).toBe(true);
    expect(canForget("deepseek", saved())).toBe(false);
    expect(canForget("ollama", saved({ provider: "ollama", keySet: false, keyHint: undefined }))).toBe(false);
  });
});

describe("the saved line", () => {
  it("names the provider, its model and its key's last four", () => {
    expect(savedLine(saved())).toBe("Claude · key ending 0000");
    expect(savedLine(saved({ model: "example-model-a" }))).toBe("Claude · example-model-a · key ending 0000");
    expect(savedLine(saved({ provider: "ollama", keySet: false, keyHint: undefined, model: "example-model-a" }))).toBe(
      "Ollama · example-model-a",
    );
    expect(savedLine(nothing)).toBe("Not chosen yet");
    expect(savedLine(undefined)).toBe("Not chosen yet");
  });

  it("reads sensibly for saved providers without a row", () => {
    expect(savedLine(saved({ provider: "claude", keySet: false, keyHint: undefined }))).toBe("Claude, on its own sign-in");
    expect(savedLine(saved({ provider: "anthropic-compatible" }))).toBe("An Anthropic-style service · key ending 0000");
    expect(providerName("some-new-provider")).toBe("some-new-provider");
  });

  it("says when what is saved isn't what the agent thinks with", () => {
    expect(savedStatus(saved())).toBeUndefined();
    expect(savedStatus(saved({ inUse: false }))).toBe("Saved, not in use: this server's own accounts come first.");
    expect(savedStatus(saved({ provider: "openai", usable: false, inUse: false }))).toBe("Saved. Your ember can't think with it yet.");
    expect(savedStatus(nothing)).toBeUndefined();
  });
});

describe("the test's words", () => {
  it("counts the models", () => {
    expect(passedLine(models)).toBe("It works · 2 models");
    expect(passedLine([{ id: "example-model-a" }])).toBe("It works · 1 model");
    expect(passedLine([])).toBe("It works");
    expect(modelLabel(models[0]!)).toBe("example-model-a");
    expect(modelLabel(models[1]!)).toBe("Example Model B");
  });

  it("finds a model by its id or name", () => {
    expect(modelsShown(models, "").map((m) => m.id)).toEqual(["example-model-a", "example-model-b"]);
    expect(modelsShown(models, " model b ").map((m) => m.id)).toEqual(["example-model-b"]);
    expect(modelsShown(models, "MODEL-A").map((m) => m.id)).toEqual(["example-model-a"]);
    expect(modelsShown(models, "none")).toEqual([]);
  });

  it("shows the server's sentence as it is", () => {
    expect(FAILED_TITLE).toBe("Didn't answer");
    expect(failureReason(new ApiError(422, "The provider refused the key."))).toBe("The provider refused the key.");
    expect(failureReason(new ApiError(0, "Can't reach Emberhut"))).toBe("Can't reach Emberhut");
    expect(failureReason("odd")).toBe("Something went wrong");
  });
});

describe("the picker", () => {
  it("opens on the saved provider's row, with its saved base URL", () => {
    expect(initialPicker(undefined).selected).toBeUndefined();
    expect(initialPicker(saved()).selected).toBe("anthropic");
    const ollama = initialPicker(saved({ provider: "ollama", baseUrl: "http://example.test:11434" }));
    expect([ollama.selected, ollama.baseUrl]).toEqual(["ollama", "http://example.test:11434"]);
    expect(initialPicker(saved({ provider: "claude" })).selected).toBeUndefined();
    expect(initialPicker(saved({ provider: "openai", usable: false })).selected).toBeUndefined();
  });

  it("doesn't select a row it can't run on", () => {
    const start = run([{ type: "select", kind: "anthropic", saved: undefined }]);
    expect(pickerReducer(start, { type: "select", kind: "openai", saved: undefined })).toBe(start);
    expect(run([{ type: "select", kind: "openai", saved: saved({ provider: "openai", usable: true }) }]).selected).toBe("openai");
  });

  it("clears what was typed, the test and the error when another row is selected", () => {
    const failedSave = pickerReducer(passed("anthropic", [{ type: "edit", field: "key", value: exampleKey }]), { type: "saveStarted" });
    const state = run(
      [
        { type: "saveFailed", reason: "The provider refused the key." },
        { type: "select", kind: "deepseek", saved: undefined },
      ],
      failedSave,
    );
    expect(state).toMatchObject({ selected: "deepseek", key: "", baseUrl: "", test: { step: "idle" }, saveError: undefined });
    expect(state.model).toBeUndefined();
  });

  it("keeps what is typed when the open row is tapped again", () => {
    const typed = run([
      { type: "select", kind: "anthropic", saved: undefined },
      { type: "edit", field: "key", value: exampleKey },
    ]);
    expect(pickerReducer(typed, { type: "select", kind: "anthropic", saved: undefined })).toBe(typed);
  });

  it("forgets a test once a field changes", () => {
    const ok = passed("anthropic", [{ type: "edit", field: "key", value: exampleKey }]);
    expect(ok.test.step).toBe("passed");
    expect(canSave(ok)).toBe(true);
    const edited = pickerReducer(ok, { type: "edit", field: "key", value: `${exampleKey}1` });
    expect(edited.test.step).toBe("idle");
    expect(edited.model).toBeUndefined();
    expect(canSave(edited)).toBe(false);

    const before = run([
      { type: "select", kind: "anthropic", saved: undefined },
      { type: "edit", field: "key", value: exampleKey },
      { type: "testStarted" },
    ]);
    const failed = pickerReducer(before, { type: "testFailed", version: before.version, reason: "The provider refused the key." });
    expect(failed.test).toEqual({ step: "failed", version: before.version, reason: "The provider refused the key." });
    expect(pickerReducer(failed, { type: "edit", field: "key", value: "x" }).test.step).toBe("idle");
  });

  it("drops a test's answer that comes after the fields changed", () => {
    const testing = run([
      { type: "select", kind: "anthropic", saved: undefined },
      { type: "edit", field: "key", value: exampleKey },
      { type: "testStarted" },
    ]);
    const asked = testing.version;
    const edited = pickerReducer(testing, { type: "edit", field: "key", value: "sk-ant-other" });
    expect(pickerReducer(edited, { type: "testPassed", version: asked, models, saved: undefined })).toBe(edited);
    const again = pickerReducer(edited, { type: "testStarted" });
    expect(pickerReducer(again, { type: "testFailed", version: asked, reason: "late" })).toBe(again);
  });

  it("starts on the saved model when it is listed, else the first", () => {
    const on = saved({ model: "example-model-b" });
    expect(preselectModel("anthropic", models, on)).toBe("example-model-b");
    expect(preselectModel("deepseek", models, on)).toBe("example-model-a");
    expect(preselectModel("anthropic", models, saved({ model: "gone" }))).toBe("example-model-a");
    expect(preselectModel("anthropic", [], on)).toBeUndefined();
    expect(passed("anthropic", [{ type: "edit", field: "key", value: exampleKey }], models, on).model).toBe("example-model-b");
  });

  it("chooses only a listed model", () => {
    const ok = passed("anthropic", [{ type: "edit", field: "key", value: exampleKey }]);
    expect(pickerReducer(ok, { type: "chooseModel", id: "example-model-b" }).model).toBe("example-model-b");
    expect(pickerReducer(ok, { type: "chooseModel", id: "not-listed" })).toBe(ok);
  });

  it("tests once every needed box is filled", () => {
    const anthropic = run([{ type: "select", kind: "anthropic", saved: undefined }]);
    expect(canTest(initialPicker(undefined))).toBe(false);
    expect(canTest(anthropic)).toBe(false);
    expect(canTest(pickerReducer(anthropic, { type: "edit", field: "key", value: "   " }))).toBe(false);
    expect(canTest(pickerReducer(anthropic, { type: "edit", field: "key", value: exampleKey }))).toBe(true);
    expect(canTest(run([{ type: "select", kind: "ollama", saved: undefined }]))).toBe(true);
    const bridged = saved({ provider: "openai-compatible", usable: true });
    const other = run([{ type: "select", kind: "openai-compatible", saved: bridged }]);
    expect(canTest(other, bridged)).toBe(false);
    expect(canTest(pickerReducer(other, { type: "edit", field: "baseUrl", value: "https://example.test/v1" }), bridged)).toBe(true);
    const testing = run([{ type: "edit", field: "key", value: exampleKey }, { type: "testStarted" }], anthropic);
    expect(canTest(testing)).toBe(false);
    expect(pickerReducer(testing, { type: "testStarted" })).toBe(testing);
  });

  it("saves with a model chosen, or when the provider lists none", () => {
    expect(canSave(passed("ollama", [], []))).toBe(true);
    const unchosen: PickerState = { ...passed("ollama", []), model: undefined };
    expect(canSave(unchosen)).toBe(false);
    const saving = pickerReducer(passed("ollama", []), { type: "saveStarted" });
    expect(saving.saving).toBe(true);
    expect(canSave(saving)).toBe(false);
    expect(canTest(saving)).toBe(false);
    expect(pickerReducer(saving, { type: "edit", field: "baseUrl", value: "x" })).toBe(saving);
    expect(pickerReducer(saving, { type: "saveFailed", reason: "The provider didn't answer." })).toMatchObject({
      saving: false,
      saveError: "The provider didn't answer.",
    });
  });

  it("keeps no key once it is saved", () => {
    const ok = pickerReducer(passed("anthropic", [{ type: "edit", field: "key", value: exampleKey }]), { type: "saveStarted" });
    const done = pickerReducer(ok, { type: "saved" });
    expect(done).toMatchObject({ key: "", saving: false, test: { step: "idle" } });
    expect(JSON.stringify(done)).not.toContain(exampleKey);
  });

  it("confirms a forget once", () => {
    const asked = run([{ type: "askForget" }]);
    expect(asked.forget).toBe("confirming");
    expect(pickerReducer(asked, { type: "cancelForget" }).forget).toBe("idle");
    const forgetting = pickerReducer(asked, { type: "forgetStarted" });
    expect(forgetting.forget).toBe("forgetting");
    expect(pickerReducer(initialPicker(undefined), { type: "forgetStarted" }).forget).toBe("idle");
    expect(pickerReducer(forgetting, { type: "forgetFailed", reason: "Can't reach Emberhut" })).toMatchObject({
      forget: "idle",
      forgetError: "Can't reach Emberhut",
    });
    expect(pickerReducer(forgetting, { type: "forgotten" }).forget).toBe("idle");
  });
});

describe("what is sent", () => {
  it("trims, leaves blank boxes out and sends the model only on a save", () => {
    const ok = passed("anthropic", [{ type: "edit", field: "key", value: `  ${exampleKey}  ` }]);
    expect(request(ok, "test")).toEqual({ provider: "anthropic", apiKey: exampleKey });
    expect(request(ok, "save")).toEqual({ provider: "anthropic", apiKey: exampleKey, model: "example-model-a" });
    expect(request(initialPicker(undefined), "test")).toBeUndefined();
  });

  it("never sends a key to Ollama, and its base URL only when typed", () => {
    const ollama = run([
      { type: "select", kind: "ollama", saved: undefined },
      { type: "edit", field: "key", value: exampleKey },
    ]);
    expect(request(ollama, "test")).toEqual({ provider: "ollama" });
    const typed = pickerReducer(ollama, { type: "edit", field: "baseUrl", value: " http://example.test:11434 " });
    expect(request(typed, "test")).toEqual({ provider: "ollama", baseUrl: "http://example.test:11434" });
  });

  it("sends another service's key only when one is typed", () => {
    const on = saved({ provider: "openai-compatible", usable: true });
    const other = run([
      { type: "select", kind: "openai-compatible", saved: on },
      { type: "edit", field: "baseUrl", value: "https://example.test/v1" },
      { type: "edit", field: "key", value: "  " },
    ]);
    expect(request(other, "test")).toEqual({ provider: "openai-compatible", baseUrl: "https://example.test/v1" });
    expect(request(pickerReducer(other, { type: "edit", field: "key", value: exampleKey }), "test")).toEqual({
      provider: "openai-compatible",
      baseUrl: "https://example.test/v1",
      apiKey: exampleKey,
    });
  });
});

describe("another service", () => {
  const another = ROWS.find((r) => r.kind === "openai-compatible")!;

  it("is one row for either API, picked while Anthropic-style runs", () => {
    expect(rowOf("anthropic-compatible")).toBe(another);
    expect(rowKinds(another)).toEqual(["openai-compatible", "anthropic-compatible"]);
    expect(canPickRow(another, undefined)).toBe(true);
    expect(rowTag(another, undefined)).toBeUndefined();
    expect(run([{ type: "select", kind: "openai-compatible", saved: undefined }]).selected).toBe("openai-compatible");
  });

  it("can't test OpenAI-style until the server says it runs", () => {
    const openAiStyle = run([
      { type: "select", kind: "openai-compatible", saved: undefined },
      { type: "edit", field: "baseUrl", value: "https://example.test/v1" },
    ]);
    expect(canTest(openAiStyle, undefined)).toBe(false);
    expect(pickerReducer(openAiStyle, { type: "testStarted" })).toBe(openAiStyle);
    expect(comingNextFor("openai-compatible")).toBe("Coming next: your ember can't think with an OpenAI-style service yet.");
    expect(canTest(openAiStyle, saved({ provider: "openai-compatible", usable: true }))).toBe(true);
  });

  it("speaks Anthropic-style with what was typed kept, and needs its key", () => {
    const state = run([
      { type: "select", kind: "openai-compatible", saved: undefined },
      { type: "edit", field: "baseUrl", value: "https://example.test" },
      { type: "speak", kind: "anthropic-compatible" },
    ]);
    expect(state.selected).toBe("anthropic-compatible");
    expect(state.baseUrl).toBe("https://example.test");
    expect(canTest(state, undefined)).toBe(false);
    const keyed = pickerReducer(state, { type: "edit", field: "key", value: exampleKey });
    expect(canTest(keyed, undefined)).toBe(true);
    expect(request(keyed, "test")).toEqual({ provider: "anthropic-compatible", baseUrl: "https://example.test", apiKey: exampleKey });
    expect(pickerReducer(keyed, { type: "select", kind: "openai-compatible", saved: undefined })).toBe(keyed);
    expect(pickerReducer(keyed, { type: "speak", kind: "deepseek" })).toBe(keyed);
  });

  it("opens a saved Anthropic-style service on its choice", () => {
    const on = saved({ provider: "anthropic-compatible", baseUrl: "https://example.test" });
    const state = initialPicker(on);
    expect([state.selected, state.baseUrl]).toEqual(["anthropic-compatible", "https://example.test"]);
    expect(keyPlaceholder("anthropic-compatible", on)).toBe("Saved key ending in 0000");
  });
});

describe("a local Ollama", () => {
  const ollama = ROWS.find((r) => r.kind === "ollama")!;
  const found = (from = initialPicker(undefined), listed = models) =>
    run([{ type: "lookStarted" }, { type: "lookFound", on: FOUND_ON, models: listed, saved: undefined }], from);

  it("is looked for with no key at the server's default address", () => {
    expect(OLLAMA_LOOK).toEqual({ provider: "ollama" });
    const looking = run([{ type: "lookStarted" }]);
    expect(pickerReducer(looking, { type: "lookStarted" })).toBe(looking);
  });

  it("tags its row with where it was found", () => {
    expect(rowTag(ollama, undefined, found().ollama)).toBe("Found on your ember's computer");
    expect(rowTag(ollama, undefined, { step: "missing" })).toBeUndefined();
    expect(rowTag(ROWS[0]!, undefined, found().ollama)).toBe("Recommended");
  });

  it("fills its models in as a passed test, ready to use", () => {
    const state = pickerReducer(found(), { type: "select", kind: "ollama", saved: undefined });
    expect(state.test).toMatchObject({ step: "passed", models });
    expect(state.model).toBe("example-model-a");
    expect(canSave(state)).toBe(true);
    expect(request(state, "save")).toEqual({ provider: "ollama", model: "example-model-a" });
    const on = saved({ provider: "ollama", keySet: false, keyHint: undefined, model: "example-model-b" });
    const openOnSaved = run([{ type: "lookStarted" }, { type: "lookFound", on: FOUND_ON, models, saved: on }], initialPicker(on));
    expect(openOnSaved.model).toBe("example-model-b");
  });

  it("checks another address anew, and fills in again back at the default", () => {
    const state = pickerReducer(found(), { type: "select", kind: "ollama", saved: undefined });
    const elsewhere = pickerReducer(state, { type: "edit", field: "baseUrl", value: "http://example.test:11434" });
    expect(elsewhere.test.step).toBe("idle");
    expect(pickerReducer(elsewhere, { type: "edit", field: "baseUrl", value: "" }).test.step).toBe("passed");
  });

  it("is no problem when none answers", () => {
    const state = run([{ type: "lookStarted" }, { type: "lookMissed" }, { type: "select", kind: "ollama", saved: undefined }]);
    expect(rowTag(ollama, undefined, state.ollama)).toBeUndefined();
    expect(state.test.step).toBe("idle");
    expect(state.saveError).toBeUndefined();
    expect(pickerReducer(found(initialPicker(undefined), []), { type: "select", kind: "ollama", saved: undefined }).test.step).toBe("idle");
  });
});
