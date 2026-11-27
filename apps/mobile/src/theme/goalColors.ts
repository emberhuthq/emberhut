// The goals' colors, by rank: orange, green, purple, cyan, pink, yellow and mint, and gray for what has no goal, as on the
// desktop. In light each is deep enough to read 3:1 on any light ground (iOS's own system colors are too light for it: the
// yellow reads 1.4:1 on paper); in dark they are iOS's dark-mode values. Local until packages/design-tokens holds them with
// the rest.

export interface GoalPalette {
  /** By rank: the first goal takes the first, round the list past the last. */
  ranks: readonly string[];
  none: string;
}

const light: GoalPalette = {
  ranks: ["#B45A00", "#21803A", "#8E3FBF", "#1F72A8", "#C8203F", "#8A6A00", "#00837C"],
  none: "#6E6E73",
};

const dark: GoalPalette = {
  ranks: ["#FF9F0A", "#30D158", "#BF5AF2", "#64D2FF", "#FF375F", "#FFD60A", "#63E6E2"],
  none: "#8E8E93",
};

export const goalColors = (scheme: "light" | "dark"): GoalPalette => (scheme === "dark" ? dark : light);
