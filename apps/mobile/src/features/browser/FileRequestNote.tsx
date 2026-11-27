import * as DocumentPicker from "expo-document-picker";
import * as ImagePicker from "expo-image-picker";
import { useState } from "react";
import { StyleSheet, View } from "react-native";
import { uploadName } from "@/features/chat/uploads";
import { refusalOf } from "@/features/chat/useUploads";
import { inBackground } from "@/log";
import { api } from "@/services";
import { useTheme } from "@/theme";
import { ActionSheet, Button, Icon, Text } from "@/ui";

/** The page asks for a file (an upload field): your own picker, the file you pick goes into the page. */
export function FileRequestNote({ onSent }: { onSent: () => void }) {
  const { colors, radius, spacing } = useTheme();
  const [choosing, setChoosing] = useState(false);
  const [sending, setSending] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  const send = async (picked: { uri: string; name?: string | null; mimeType?: string | null } | undefined) => {
    if (!picked) return;
    const name = uploadName(picked, Date.now());
    const refused = refusalOf(picked.uri);
    if (refused) {
      setFailed(refused);
      return;
    }
    setSending(name);
    setFailed(null);
    try {
      await api.browser.upload(picked.uri, name);
      onSent();
    } catch {
      setFailed("It didn't go up. Try again.");
    } finally {
      setSending(null);
    }
  };
  const photo = async () => {
    const r = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ["images", "videos"], quality: 1 });
    if (!r.canceled) await send(r.assets[0] && { uri: r.assets[0].uri, name: r.assets[0].fileName, mimeType: r.assets[0].mimeType });
  };
  const file = async () => {
    const r = await DocumentPicker.getDocumentAsync({ copyToCacheDirectory: true });
    if (!r.canceled) await send(r.assets[0]);
  };

  return (
    <View
      style={[
        styles.row,
        { gap: spacing.sm, padding: spacing.sm, paddingLeft: spacing.md, borderRadius: radius.lg, backgroundColor: colors.surface },
      ]}
    >
      <Icon name="attach" color={colors.working} />
      <Text variant="callout" weight="500" style={styles.fill}>
        {sending ? `Sending ${sending}…` : (failed ?? "The page wants a file")}
      </Text>
      <Button label="Choose…" onPress={() => setChoosing(true)} loading={sending !== null} />
      <ActionSheet
        visible={choosing}
        actions={[
          { label: "Photos", icon: "images-outline", onPress: () => inBackground(photo(), "FileRequestNote: photo") },
          { label: "Files", icon: "folder-outline", onPress: () => inBackground(file(), "FileRequestNote: file") },
        ]}
        onClose={() => setChoosing(false)}
      />
    </View>
  );
}

const styles = StyleSheet.create({ row: { flexDirection: "row", alignItems: "center" }, fill: { flex: 1 } });
