import { useState } from "react";
import { Pressable, StyleSheet, TextInput, View } from "react-native";
import type { BrowserInfo, BrowserInput } from "@/api";
import { minTap, useTheme } from "@/theme";
import { ActionSheet, Button, Icon, Text, type IconName, type SheetAction } from "@/ui";
import { activeTab, address, isSecure, site } from "./browser";

export interface BrowserBarProps {
  info: BrowserInfo;
  mine: boolean;
  act: (input: BrowserInput) => void;
  onTakeOver: () => void;
  /** While you drive: the keyboard row, the shortcuts. */
  typing: boolean;
  onTyping: () => void;
  onPaste: () => void;
  onShortcuts: () => void;
}

/** At the foot, like a phone browser's: the address, back and forward, the tabs, and take over or give back. */
export function BrowserBar({ info, mine, act, onTakeOver, typing, onTyping, onPaste, onShortcuts }: BrowserBarProps) {
  const { colors, radius, spacing, type } = useTheme();
  const [editing, setEditing] = useState(false);
  const [typed, setTyped] = useState("");
  const [tabs, setTabs] = useState(false);

  const tabActions: SheetAction[] = [
    ...(info.tabs ?? []).map((t) => ({
      label: `${t.active ? "✓ " : ""}${t.title || t.url}`,
      icon: "browsers-outline" as const,
      onPress: () => act({ kind: "tab", id: t.id }),
    })),
    { label: "New tab", icon: "add", onPress: () => act({ kind: "newtab" }) },
    ...(activeTab(info)
      ? [
          {
            label: "Close this tab",
            icon: "close" as const,
            destructive: true,
            onPress: () => act({ kind: "closetab", id: activeTab(info)!.id }),
          },
        ]
      : []),
  ];

  return (
    <View
      style={{
        gap: spacing.sm,
        paddingHorizontal: spacing.md,
        paddingTop: spacing.sm,
        paddingBottom: spacing.xs,
        backgroundColor: colors.surface,
      }}
    >
      {editing ? (
        <View style={[styles.row, { gap: spacing.sm }]}>
          <TextInput
            value={typed}
            onChangeText={setTyped}
            autoFocus
            selectTextOnFocus
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
            returnKeyType="go"
            onSubmitEditing={() => {
              act({ kind: "go", url: typed });
              setEditing(false);
            }}
            placeholder="Search or enter website"
            placeholderTextColor={colors.textTertiary}
            accessibilityLabel="Address"
            style={[
              type.callout,
              styles.fill,
              {
                minHeight: 40,
                paddingHorizontal: spacing.md,
                borderRadius: radius.pill,
                color: colors.text,
                backgroundColor: colors.surfaceRaised,
              },
            ]}
          />
          <Button label="Cancel" variant="plain" onPress={() => setEditing(false)} />
        </View>
      ) : (
        <Pressable
          onPress={() => {
            setTyped(address(info));
            setEditing(true);
          }}
          disabled={!mine}
          accessibilityRole="button"
          accessibilityLabel={`Address ${site(info)}`}
          accessibilityHint={mine ? "Type an address" : undefined}
          style={[
            styles.row,
            {
              gap: spacing.sm,
              minHeight: 40,
              paddingHorizontal: spacing.md,
              borderRadius: radius.pill,
              backgroundColor: colors.surfaceRaised,
            },
          ]}
        >
          <Icon name={isSecure(info) ? "lock-closed" : "globe-outline"} size={14} color={colors.textSecondary} />
          <Text variant="callout" weight="500" numberOfLines={1} style={styles.fill}>
            {site(info)}
          </Text>
          {mine && <BarIcon icon="refresh" label="Reload" onPress={() => act({ kind: "reload" })} />}
        </Pressable>
      )}
      <View style={[styles.row, { gap: spacing.xs }]}>
        <BarIcon icon="chevron-back" label="Back" onPress={() => act({ kind: "back" })} disabled={!mine} />
        <BarIcon icon="chevron-forward" label="Forward" onPress={() => act({ kind: "forward" })} disabled={!mine} />
        <View style={styles.fill} />
        {mine && (
          <>
            <BarIcon
              icon={typing ? "chevron-down-circle-outline" : "keypad-outline"}
              label={typing ? "Hide the keyboard row" : "Type into the page"}
              onPress={onTyping}
            />
            <BarIcon icon="clipboard-outline" label="Paste from the phone" onPress={onPaste} />
            <BarIcon icon="options-outline" label="Shortcuts" onPress={onShortcuts} />
          </>
        )}
        <BarIcon icon="copy-outline" label={`Tabs, ${info.tabs?.length ?? 1}`} onPress={() => setTabs(true)} disabled={!mine} />
        <View style={styles.fill} />
        {mine ? (
          <Button label="Give back" icon="arrow-undo" onPress={() => act({ kind: "handback" })} />
        ) : (
          <Button label="Take over" icon="hand-left-outline" onPress={onTakeOver} />
        )}
      </View>
      <ActionSheet visible={tabs} title="Tabs" actions={tabActions} onClose={() => setTabs(false)} />
    </View>
  );
}

function BarIcon({ icon, label, onPress, disabled = false }: { icon: IconName; label: string; onPress: () => void; disabled?: boolean }) {
  const { colors } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      style={styles.icon}
    >
      <Icon name={icon} size={22} color={disabled ? colors.textTertiary : colors.text} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center" },
  fill: { flex: 1 },
  icon: { width: minTap - 4, height: minTap, alignItems: "center", justifyContent: "center" },
});
