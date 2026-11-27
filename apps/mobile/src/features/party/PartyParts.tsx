import { View } from "react-native";
import type { PartyPersona, PartyStatus } from "@/api";
import { useTheme } from "@/theme";
import { Text } from "@/ui";
import { initial, personaColor, statusTone } from "./party";

/** A persona's initial in a circle of their colour. */
export function Initial({ name, color, size = 28 }: { name: string; color: string; size?: number }) {
  return (
    <View
      style={{ width: size, height: size, borderRadius: size / 2, backgroundColor: color, alignItems: "center", justifyContent: "center" }}
    >
      <Text weight="700" style={{ color: "#FFFFFF", fontSize: size * 0.48, lineHeight: size * 0.6 }}>
        {initial(name)}
      </Text>
    </View>
  );
}

/** The cast as overlapping coloured initials. */
export function CastDots({ personas, size = 22 }: { personas: PartyPersona[]; size?: number }) {
  const { colors, scheme } = useTheme();
  return (
    <View style={{ flexDirection: "row" }} accessibilityLabel={personas.map((p) => p.name).join(", ")}>
      {personas.map((p, i) => (
        <View
          key={p.id}
          style={{ marginLeft: i === 0 ? 0 : -size * 0.3, borderRadius: size, borderWidth: 1.5, borderColor: colors.surface }}
        >
          <Initial name={p.name} color={personaColor(p.color, scheme)} size={size} />
        </View>
      ))}
    </View>
  );
}

/** The status's small dot: talking green, paused orange, ended grey. */
export function StatusDot({ status }: { status: PartyStatus }) {
  const { colors } = useTheme();
  const tone = statusTone(status);
  const color = tone === "done" ? colors.done : tone === "warning" ? colors.warning : colors.textTertiary;
  return <View style={{ width: 7, height: 7, borderRadius: 4, backgroundColor: color }} />;
}
