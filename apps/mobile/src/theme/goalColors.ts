// The goals' colors, by rank: iOS's system orange, green, purple, cyan, pink, yellow and mint, each with its dark-mode
// value, and gray for what has no goal. Local until packages/design-tokens holds them with the rest.

export interface GoalPalette {
  /** By rank: the first goal takes the first, round the list past the last. */
  ranks: readonly string[];
  none: string;
}

const light: GoalPalette = {
  ranks: ["#FF9500", "#34C759", "#AF52DE", "#32ADE6", "#FF2D55", "#FFCC00", "#00C7BE"],
  none: "#8E8E93",
};

const dark: GoalPalette = {
  ranks: ["#FF9F0A", "#30D158", "#BF5AF2", "#64D2FF", "#FF375F", "#FFD60A", "#63E6E2"],
  none: "#8E8E93",
};

export const goalColors = (scheme: "light" | "dark"): GoalPalette => (scheme === "dark" ? dark : light);
