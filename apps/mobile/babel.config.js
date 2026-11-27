// babel-preset-expo resolves the "@/" alias from tsconfig's paths.
module.exports = function (api) {
  api.cache(true);
  return { presets: ["babel-preset-expo"] };
};
