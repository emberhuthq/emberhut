import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useReducer, useRef, useState, type Dispatch } from "react";
import { Alert, Pressable, ScrollView, View } from "react-native";
import type { AgentProvider, ProviderModel, ProviderRequest } from "@/api";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { useAgentProvider } from "@/state/queries";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import { Button, Chip, ErrorState, Icon, ListRow, Loading, Screen, Section, Segmented, Text, TextField } from "@/ui";
import {
  FAILED_TITLE,
  FOUND_ON,
  LOOK_MS,
  MODELS_FINDABLE,
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
  request,
  rowKinds,
  rowOf,
  rowTag,
  savedLine,
  savedStatus,
  speaksLabel,
  type PickerAction,
  type PickerState,
  type ProviderRow,
  type TestState,
} from "./credentials";
import { ProviderLogo } from "./ProviderLogo";

const FOOTER =
  "Your key stays on the computer your ember lives on, in a file only your user account can read. It's sent only to the provider you pick.";

/** The stack's page: the picker, closed once a provider is saved. */
export function CredentialsScreen({ navigation }: RootScreenProps<"Credentials">) {
  return (
    <Screen scroll>
      <Credentials onDone={() => navigation.goBack()} />
    </Screen>
  );
}

/**
 * What your ember thinks with: one provider picked from a list, its key (or address) typed, tested with the provider, a
 * model picked from what it lists, then saved on the server. Usable outside the stack: `onDone` runs once it is saved.
 */
export function Credentials({ onDone }: { onDone: () => void }) {
  const provider = useAgentProvider();
  if (provider.isPending) return <Loading />;
  if (provider.isError)
    return (
      <ErrorState message={provider.error.message} onRetry={() => inBackground(provider.refetch(), "Credentials: refetch provider")} />
    );
  return <Picker saved={provider.data} onDone={onDone} />;
}

function Picker({ saved, onDone }: { saved: AgentProvider; onDone: () => void }) {
  const { spacing } = useTheme();
  const client = useQueryClient();
  const [state, dispatch] = useReducer(pickerReducer, saved, initialPicker);
  // A request carries the key typed now: gcTime 0 drops it from the mutation cache as soon as nothing shows it.
  const test = useMutation({ mutationFn: (body: ProviderRequest) => api.agentProvider.test(body), gcTime: 0 });
  const keep = (next: AgentProvider) => client.setQueryData(queryKeys.agentProvider, next);
  const save = useMutation({ mutationFn: (body: ProviderRequest) => api.agentProvider.save(body), gcTime: 0, onSuccess: keep });
  const forget = useMutation({ mutationFn: () => api.agentProvider.forget(), onSuccess: keep });
  // The saved provider as it is when a late answer comes, for the model it starts on.
  const savedNow = useRef(saved);
  useEffect(() => {
    savedNow.current = saved;
  }, [saved]);

  // Once per opening, quietly: does a local Ollama answer at the server's default address? No answer is no problem.
  useEffect(() => {
    let open = true;
    dispatch({ type: "lookStarted" });
    api.agentProvider.test(OLLAMA_LOOK, LOOK_MS).then(
      (answer) => {
        if (open) dispatch({ type: "lookFound", on: FOUND_ON, models: answer.models, saved: savedNow.current });
      },
      () => {
        if (open) dispatch({ type: "lookMissed" });
      },
    );
    return () => {
      open = false;
    };
  }, []);

  const runTest = () => {
    const body = request(state, "test");
    if (!body || !canTest(state, saved)) return;
    const { version } = state;
    dispatch({ type: "testStarted", saved });
    test.mutate(body, {
      onSuccess: (answer) => dispatch({ type: "testPassed", version, models: answer.models, saved }),
      onError: (error) => dispatch({ type: "testFailed", version, reason: failureReason(error) }),
      onSettled: () => test.reset(),
    });
  };
  const use = () => {
    const body = request(state, "save");
    if (!body || !canSave(state)) return;
    dispatch({ type: "saveStarted" });
    save.mutate(body, {
      onSuccess: () => {
        dispatch({ type: "saved" });
        save.reset();
        onDone();
      },
      onError: (error) => {
        dispatch({ type: "saveFailed", reason: failureReason(error) });
        save.reset();
      },
    });
  };
  const askForget = () => {
    dispatch({ type: "askForget" });
    Alert.alert(
      "Forget this key?",
      "The server removes it, and your ember goes back to its own sign-in.",
      [
        { text: "Cancel", style: "cancel", onPress: () => dispatch({ type: "cancelForget" }) },
        {
          text: "Forget",
          style: "destructive",
          onPress: () => {
            dispatch({ type: "forgetStarted" });
            forget.mutate(undefined, {
              onSuccess: () => dispatch({ type: "forgotten" }),
              onError: (error) => dispatch({ type: "forgetFailed", reason: failureReason(error) }),
            });
          },
        },
      ],
      { cancelable: true, onDismiss: () => dispatch({ type: "cancelForget" }) },
    );
  };

  const status = savedStatus(saved);
  return (
    <View style={{ gap: spacing.lg }}>
      <Section title="Thinks with now">
        <ListRow key="saved" icon="flame-outline" title={savedLine(saved)} subtitle={status} />
      </Section>
      <Section footer={FOOTER}>
        {ROWS.map((row) => (
          <ProviderRowView
            key={row.kind}
            row={row}
            saved={saved}
            state={state}
            dispatch={dispatch}
            onTest={runTest}
            onUse={use}
            onForget={askForget}
          />
        ))}
      </Section>
    </View>
  );
}

interface RowProps {
  row: ProviderRow;
  saved: AgentProvider;
  state: PickerState;
  dispatch: Dispatch<PickerAction>;
  onTest: () => void;
  onUse: () => void;
  onForget: () => void;
}

/** A radio-like row: its mark, name and tag, one line; the fields open under it once it is selected. */
function ProviderRowView({ row, saved, state, dispatch, onTest, onUse, onForget }: RowProps) {
  const { colors, spacing } = useTheme();
  const runs = canPickRow(row, saved);
  const selected = rowOf(state.selected) === row;
  const tag = rowTag(row, saved, state.ollama);
  return (
    <View style={{ backgroundColor: colors.surface }}>
      <Pressable
        onPress={() => dispatch({ type: "select", kind: row.kind, saved })}
        disabled={!runs}
        accessibilityRole="radio"
        accessibilityLabel={[row.name, tag, row.line].filter(Boolean).join(", ")}
        accessibilityState={{ checked: selected, disabled: !runs }}
        style={({ pressed }) => ({
          flexDirection: "row",
          alignItems: "flex-start",
          gap: spacing.md,
          paddingHorizontal: spacing.lg,
          paddingVertical: spacing.md,
          backgroundColor: pressed ? colors.surfaceRaised : colors.surface,
          opacity: runs ? 1 : 0.5,
        })}
      >
        <Icon name={selected ? "radio-button-on" : "radio-button-off"} color={selected ? colors.accent : colors.textTertiary} />
        <ProviderLogo kind={row.kind} color={colors.textSecondary} />
        <View style={{ flex: 1, minWidth: 0, gap: 2 }}>
          <View style={{ flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: spacing.sm }}>
            <Text variant="headline">{row.name}</Text>
            {tag ? <Chip label={tag} tone={runs ? "needsYou" : "muted"} /> : null}
          </View>
          <Text variant="footnote" tone="secondary">
            {row.line}
          </Text>
        </View>
      </Pressable>
      {selected ? <RowFields row={row} saved={saved} state={state} dispatch={dispatch} onTest={onTest} onUse={onUse} /> : null}
      {rowKinds(row).some((kind) => canForget(kind, saved)) ? (
        <View style={{ paddingHorizontal: spacing.lg, paddingBottom: spacing.sm, alignItems: "flex-start" }}>
          <Button
            label="Forget this key"
            variant="plain"
            icon="trash-outline"
            onPress={onForget}
            loading={state.forget === "forgetting"}
            disabled={state.saving || state.forget !== "idle"}
          />
          {state.forgetError ? (
            <Text variant="footnote" tone="danger">
              {state.forgetError}
            </Text>
          ) : null}
        </View>
      ) : null}
    </View>
  );
}

/** The open row: its hint, its boxes, "Test it", then the test's words, the models and "Use this". */
function RowFields({ row, saved, state, dispatch, onTest, onUse }: Omit<RowProps, "onForget">) {
  const { spacing } = useTheme();
  const { test } = state;
  const kind = state.selected ?? row.kind;
  return (
    <View style={{ paddingHorizontal: spacing.lg, paddingBottom: spacing.md, gap: spacing.sm }}>
      {row.hint ? (
        <Text variant="footnote" tone="tertiary">
          {row.hint}
        </Text>
      ) : null}
      {row.also !== undefined ? (
        <View style={{ gap: spacing.xs }}>
          <Text variant="footnote" tone="secondary">
            It speaks
          </Text>
          <Segmented
            options={rowKinds(row).map((value) => ({ value, label: speaksLabel(value) }))}
            value={kind}
            onChange={(value) => dispatch({ type: "speak", kind: value })}
            accessibilityLabel="It speaks"
          />
          {canRunOn(kind, saved) ? null : (
            <Text variant="footnote" tone="tertiary">
              {comingNextFor(kind)}
            </Text>
          )}
        </View>
      ) : null}
      {fieldsOf(kind).map((field) =>
        field.name === "key" ? (
          <TextField
            key="key"
            value={state.key}
            onChangeText={(value) => dispatch({ type: "edit", field: "key", value, saved })}
            placeholder={keyPlaceholder(kind, saved)}
            accessibilityLabel={`${row.name} API key`}
            secureTextEntry
            autoCapitalize="none"
            autoCorrect={false}
            autoComplete="off"
            importantForAutofill="no"
            editable={!state.saving}
          />
        ) : (
          <TextField
            key="baseUrl"
            value={state.baseUrl}
            onChangeText={(value) => dispatch({ type: "edit", field: "baseUrl", value, saved })}
            placeholder={field.placeholder}
            accessibilityLabel={`${row.name} address`}
            keyboardType="url"
            autoCapitalize="none"
            autoCorrect={false}
            editable={!state.saving}
          />
        ),
      )}
      <Button
        label="Test it"
        variant="secondary"
        icon="pulse-outline"
        onPress={onTest}
        disabled={!canTest(state, saved)}
        loading={test.step === "testing"}
      />
      <TestResult test={test} />
      {test.step === "passed" ? (
        <>
          {test.models.length > 0 ? (
            <ModelPicker
              key={test.version}
              models={test.models}
              chosen={state.model}
              onChoose={(id) => dispatch({ type: "chooseModel", id })}
            />
          ) : null}
          <Button label="Use this" icon="checkmark" onPress={onUse} disabled={!canSave(state)} loading={state.saving} />
        </>
      ) : null}
      {state.saveError ? (
        <Text variant="footnote" tone="danger">
          {state.saveError}
        </Text>
      ) : null}
    </View>
  );
}

/** A passed test's words, with the provider's answer as a bubble when there is one; a failed one's title and the server's sentence. */
function TestResult({ test }: { test: TestState }) {
  const { colors, radius, spacing } = useTheme();
  if (test.step === "passed") {
    return (
      <View style={{ gap: spacing.sm }}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.xs }}>
          <Icon name="checkmark-circle" size={18} color={colors.done} />
          <Text variant="callout" tone="done" weight="600">
            {passedLine(test.models)}
          </Text>
        </View>
        {test.answer ? (
          <View
            style={{
              alignSelf: "flex-start",
              maxWidth: "85%",
              backgroundColor: colors.surfaceRaised,
              borderRadius: radius.lg,
              paddingHorizontal: spacing.md,
              paddingVertical: spacing.sm,
            }}
          >
            <Text variant="callout">{test.answer}</Text>
          </View>
        ) : null}
      </View>
    );
  }
  if (test.step === "failed") {
    return (
      <View style={{ gap: 2 }}>
        <Text variant="callout" tone="danger" weight="600">
          {FAILED_TITLE}
        </Text>
        <Text variant="footnote" tone="danger">
          {test.reason}
        </Text>
      </View>
    );
  }
  return null;
}

/** The provider's models to pick one from: a short scrolling list, with a box to find one in when it is long. */
function ModelPicker({ models, chosen, onChoose }: { models: ProviderModel[]; chosen?: string; onChoose: (id: string) => void }) {
  const { colors, radius, spacing } = useTheme();
  const [query, setQuery] = useState("");
  const shown = modelsShown(models, query);
  return (
    <View style={{ gap: spacing.sm }}>
      {models.length > MODELS_FINDABLE ? (
        <TextField
          value={query}
          onChangeText={setQuery}
          placeholder="Find a model"
          accessibilityLabel="Find a model"
          autoCapitalize="none"
          autoCorrect={false}
        />
      ) : null}
      <ScrollView
        nestedScrollEnabled
        keyboardShouldPersistTaps="handled"
        style={{ maxHeight: 240, borderRadius: radius.md, backgroundColor: colors.surfaceRaised }}
        accessibilityRole="radiogroup"
        accessibilityLabel="Model"
      >
        {shown.map((model) => {
          const on = model.id === chosen;
          return (
            <Pressable
              key={model.id}
              onPress={() => onChoose(model.id)}
              accessibilityRole="radio"
              accessibilityState={{ checked: on }}
              accessibilityLabel={modelLabel(model)}
              style={({ pressed }) => ({
                flexDirection: "row",
                alignItems: "center",
                gap: spacing.sm,
                paddingHorizontal: spacing.md,
                paddingVertical: spacing.sm,
                minHeight: 40,
                backgroundColor: pressed ? colors.border : undefined,
              })}
            >
              <Icon name={on ? "checkmark-circle" : "ellipse-outline"} size={18} color={on ? colors.accent : colors.textTertiary} />
              <View style={{ flex: 1, minWidth: 0 }}>
                <Text variant="callout" weight={on ? "600" : undefined}>
                  {modelLabel(model)}
                </Text>
                {model.name ? (
                  <Text variant="caption" tone="tertiary">
                    {model.id}
                  </Text>
                ) : null}
              </View>
            </Pressable>
          );
        })}
        {shown.length === 0 ? (
          <Text variant="footnote" tone="secondary" style={{ padding: spacing.md }}>
            No model has that in its name.
          </Text>
        ) : null}
      </ScrollView>
    </View>
  );
}
