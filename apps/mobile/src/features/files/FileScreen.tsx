import { useQuery } from "@tanstack/react-query";
import { useVideoPlayer, VideoView } from "expo-video";
import { useCallback, useLayoutEffect, useState } from "react";
import { Pressable, ScrollView, StyleSheet, View } from "react-native";
import { isServerPath } from "@/api";
import { inBackground } from "@/log";
import type { RootScreenProps } from "@/navigation/types";
import { api } from "@/services";
import { queryKeys } from "@/state/query";
import { monoFont, useTheme } from "@/theme";
import { Button, EmptyState, ErrorState, Icon, Loading, Screen, Text } from "@/ui";
import { openOutside, readText, share } from "./fileOut";
import { reportTitle, viewerKind } from "./files";
import { ReportView } from "./ReportView";
import { ServerImage } from "./ServerImage";

/** A server file full screen: a picture, a video, a report; anything else opens in its own app or leaves by Share. */
export function FileScreen({ route, navigation }: RootScreenProps<"File">) {
  const { path, name } = route.params;
  const { colors } = useTheme();
  const kind = viewerKind(name);
  const [problem, setProblem] = useState<string | null>(null);
  const run = useCallback(
    (action: (path: string, name: string) => Promise<void>) => () => {
      setProblem(null);
      action(path, name).catch((cause: unknown) => setProblem(cause instanceof Error ? cause.message : "It didn't open"));
    },
    [path, name],
  );
  const setTitle = useCallback((title: string) => navigation.setOptions({ title }), [navigation]);

  // A sheet, as iOS shows a document: Done where iOS puts it (top right, whatever the report's own direction), Share on
  // the left, the title on one line cut in the middle. A swipe down, or Android's back, closes it too.
  useLayoutEffect(() => {
    navigation.setOptions({
      headerTitle: ({ children }) => (
        <Text variant="headline" numberOfLines={1} ellipsizeMode="middle" style={styles.title}>
          {children}
        </Text>
      ),
      headerLeft: () => (
        <Pressable onPress={run(share)} accessibilityRole="button" accessibilityLabel={`Share ${name}`} hitSlop={12}>
          <Icon name="share-outline" color={colors.accent} />
        </Pressable>
      ),
      headerRight: () => (
        <Pressable onPress={() => navigation.goBack()} accessibilityRole="button" accessibilityLabel="Done" hitSlop={12}>
          <Text weight="600" style={{ color: colors.accent }}>
            Done
          </Text>
        </Pressable>
      ),
    });
  }, [navigation, run, name, colors.accent]);

  if (!isServerPath(path)) {
    return (
      <Screen>
        <EmptyState icon="alert-circle-outline" title={name} message="This file isn't on Emberhut's server, so it isn't opened here." />
      </Screen>
    );
  }

  return (
    <Screen padded={false} edges={["bottom"]}>
      {kind === "image" ? (
        <ServerImage path={path} resizeMode="contain" style={styles.fill} accessibilityLabel={name} />
      ) : kind === "video" ? (
        <Video path={path} />
      ) : kind === "html" ? (
        <Report path={path} onTitle={setTitle} />
      ) : kind === "text" ? (
        <TextFile path={path} />
      ) : (
        <EmptyState
          icon="document-outline"
          title={name}
          message="Open it in the app Android picks for it, or share it on."
          action={
            <View style={styles.actions}>
              <Button label="Open" icon="open-outline" onPress={run(openOutside)} />
              <Button label="Share" icon="share-social-outline" variant="secondary" onPress={run(share)} />
            </View>
          }
        />
      )}
      {problem ? (
        <Text tone="danger" align="center" style={styles.problem}>
          {problem}
        </Text>
      ) : null}
    </Screen>
  );
}

function Video({ path }: { path: string }) {
  const player = useVideoPlayer(api.files.source(path), (p) => p.play());
  return <VideoView player={player} style={styles.fill} contentFit="contain" nativeControls fullscreenOptions={{ enable: true }} />;
}

function Report({ path, onTitle }: { path: string; onTitle: (title: string) => void }) {
  const page = useQuery({ queryKey: queryKeys.fileText(path), queryFn: () => readText(path), staleTime: Infinity });
  const title = page.data === undefined ? null : reportTitle(page.data);
  useLayoutEffect(() => {
    if (title) onTitle(title);
  }, [title, onTitle]);
  if (page.isError)
    return <ErrorState message="The report didn't load." onRetry={() => inBackground(page.refetch(), "FileScreen: refetch")} />;
  if (page.data === undefined) return <Loading label="Opening the report" />;
  return <ReportView html={page.data} />;
}

function TextFile({ path }: { path: string }) {
  const { spacing } = useTheme();
  const file = useQuery({ queryKey: queryKeys.fileText(path), queryFn: () => readText(path), staleTime: Infinity });
  if (file.isError)
    return <ErrorState message="The file didn't load." onRetry={() => inBackground(file.refetch(), "FileScreen: refetch")} />;
  if (file.data === undefined) return <Loading />;
  return (
    <ScrollView contentContainerStyle={{ padding: spacing.lg }}>
      <Text selectable variant="footnote" style={styles.mono}>
        {file.data}
      </Text>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  title: { maxWidth: 220 },
  fill: { flex: 1 },
  actions: { flexDirection: "row", gap: 12 },
  problem: { padding: 12 },
  mono: { fontFamily: monoFont },
});
