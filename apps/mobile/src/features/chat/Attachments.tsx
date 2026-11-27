import { useNavigation } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import { Pressable, StyleSheet, View } from "react-native";
import type { ChatFile } from "@/api";
import { readText } from "@/features/files/fileOut";
import { isReport, reportTitle, stem } from "@/features/files/files";
import { ServerImage } from "@/features/files/ServerImage";
import { queryKeys } from "@/state/query";
import { useTheme } from "@/theme";
import { Icon, Text } from "@/ui";

/** A file as a message or a step carries it; a step's have no server path of their own. */
export type ShownFile = Pick<ChatFile, "name" | "url" | "kind">;

/**
 * What a message carries: pictures inline, HTML reports as cards, videos and other files as chips. A tap opens one full
 * screen. Video frames sent along for Emberhut (" · frame ") aren't shown.
 */
export function Attachments({ files, align = "left" }: { files: readonly ShownFile[]; align?: "left" | "right" }) {
  const { spacing } = useTheme();
  const shown = files.filter((f) => !f.name.includes(" · frame "));
  if (shown.length === 0) return null;
  const pictures = shown.filter((f) => f.kind === "image" && !isReport(f.name));
  const others = shown.filter((f) => !pictures.includes(f));
  return (
    <View style={{ gap: spacing.sm, alignItems: align === "right" ? "flex-end" : "flex-start" }}>
      {pictures.length > 0 && (
        <View style={[styles.wrap, { gap: spacing.xs, justifyContent: align === "right" ? "flex-end" : "flex-start" }]}>
          {pictures.map((f) => (
            <Picture key={f.url} file={f} single={pictures.length === 1} />
          ))}
        </View>
      )}
      {others.map((f) => (isReport(f.name) ? <ReportCard key={f.url} file={f} /> : <FileChip key={f.url} file={f} />))}
    </View>
  );
}

function useOpen(file: ShownFile) {
  const navigation = useNavigation();
  return () => navigation.navigate("File", { path: file.url, name: file.name, kind: file.kind });
}

function Picture({ file, single }: { file: ShownFile; single: boolean }) {
  const { radius } = useTheme();
  const open = useOpen(file);
  const size = single ? 220 : 120;
  return (
    <Pressable onPress={open} accessibilityRole="imagebutton" accessibilityLabel={`Picture ${file.name}`}>
      <ServerImage path={file.url} style={{ width: size, height: size, borderRadius: radius.md }} />
    </Pressable>
  );
}

function FileChip({ file }: { file: ShownFile }) {
  const { colors, radius, spacing } = useTheme();
  const open = useOpen(file);
  const video = file.kind === "video";
  return (
    <Pressable
      onPress={open}
      accessibilityRole="button"
      accessibilityLabel={`${video ? "Video" : "File"} ${file.name}`}
      style={({ pressed }) => [
        styles.chip,
        {
          gap: spacing.sm,
          paddingHorizontal: spacing.md,
          paddingVertical: spacing.sm,
          borderRadius: radius.md,
          backgroundColor: colors.surfaceRaised,
          opacity: pressed ? 0.7 : 1,
        },
      ]}
    >
      <Icon name={video ? "play-circle-outline" : "document-outline"} size={18} color={colors.textSecondary} />
      <Text variant="footnote" numberOfLines={1} ellipsizeMode="middle" style={styles.shrink}>
        {file.name}
      </Text>
    </Pressable>
  );
}

/** An HTML report: its page's title, a small "Report" label and the file's name; a tap opens it. */
function ReportCard({ file }: { file: ShownFile }) {
  const { colors, radius, spacing } = useTheme();
  const open = useOpen(file);
  const page = useQuery({ queryKey: queryKeys.fileText(file.url), queryFn: () => readText(file.url), staleTime: Infinity });
  const title = (page.data !== undefined ? reportTitle(page.data) : null) ?? stem(file.name);
  return (
    <Pressable
      onPress={open}
      accessibilityRole="button"
      accessibilityLabel={`Report: ${title}`}
      style={({ pressed }) => [
        styles.report,
        {
          gap: spacing.md,
          padding: spacing.sm + 2,
          borderRadius: radius.md,
          borderColor: colors.border,
          backgroundColor: colors.surface,
          opacity: pressed ? 0.8 : 1,
        },
      ]}
    >
      <View style={[styles.reportIcon, { borderRadius: radius.sm, backgroundColor: colors.surfaceRaised }]}>
        <Icon name="newspaper-outline" size={20} color={colors.accent} />
      </View>
      <View style={styles.shrink}>
        <Text variant="callout" weight="600" numberOfLines={2}>
          {title}
        </Text>
        <View style={[styles.chip, { gap: spacing.xs }]}>
          <Text variant="caption" tone="secondary" weight="600">
            Report
          </Text>
          <Text variant="caption" tone="tertiary" numberOfLines={1} ellipsizeMode="middle" style={styles.shrink}>
            {file.name}
          </Text>
        </View>
      </View>
      <Icon name="chevron-forward" size={16} color={colors.textTertiary} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  wrap: { flexDirection: "row", flexWrap: "wrap" },
  chip: { flexDirection: "row", alignItems: "center", maxWidth: 300 },
  shrink: { flexShrink: 1 },
  report: { flexDirection: "row", alignItems: "center", borderWidth: StyleSheet.hairlineWidth, maxWidth: 340, alignSelf: "stretch" },
  reportIcon: { width: 40, height: 40, alignItems: "center", justifyContent: "center" },
});
