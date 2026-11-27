// @ts-check
// Release signing for the generated android/ project. Gradle reads the keystore's properties file (storeFile, storePassword,
// keyAlias, keyPassword) from the path in EMBERHUT_ANDROID_KEYSTORE_PROPERTIES (the environment, else apps/mobile/.env) when it
// builds, so no secret ever passes through the Expo config. Without it, release builds keep the template's debug signing,
// and bundleRelease (what goes to Google Play) refuses to run.
const { withAppBuildGradle } = require("expo/config-plugins");

const marker = "// Emberhut release signing";

/** The four properties the keystore's file must have. */
const signingProperties = ["storeFile", "storePassword", "keyAlias", "keyPassword"];

/**
 * The .env line naming the properties file: the path is all after `=`, spaces included, without the quotes around it.
 * A Groovy slashy string takes it as it is (no `/` in it); Java's and JavaScript's regex read it alike.
 */
const envLine = String.raw`^\s*EMBERHUT_ANDROID_KEYSTORE_PROPERTIES\s*=\s*(["']?)(.+?)\1\s*$`;

const loader = `${marker} (plugins/withReleaseSigning.js)
def emberhutSigningPath = System.getenv("EMBERHUT_ANDROID_KEYSTORE_PROPERTIES")
def emberhutEnvFile = file("../../.env")
if (!emberhutSigningPath && emberhutEnvFile.exists()) {
    emberhutEnvFile.eachLine { line ->
        def found = line =~ /${envLine}/
        if (found) emberhutSigningPath = found[0][2]
    }
}
def emberhutSigning = null
if (emberhutSigningPath) {
    def propertiesFile = file(emberhutSigningPath)
    if (!propertiesFile.exists()) throw new GradleException("EMBERHUT_ANDROID_KEYSTORE_PROPERTIES names a missing file: " + emberhutSigningPath)
    emberhutSigning = new Properties()
    propertiesFile.withInputStream { emberhutSigning.load(it) }
    for (name in [${signingProperties.map((p) => `"${p}"`).join(", ")}]) {
        if (!emberhutSigning.getProperty(name)?.trim()) throw new GradleException(emberhutSigningPath + " has no " + name)
    }
    def store = new File(emberhutSigning.getProperty("storeFile").trim())
    emberhutSigning.setProperty("storeFile", (store.absolute ? store : new File(propertiesFile.parentFile, store.path)).path)
}
// A bundle for Google Play must carry the upload key: never let one go out with the debug key.
gradle.taskGraph.whenReady { graph ->
    if (emberhutSigning == null && graph.allTasks.any { it.name == "bundleRelease" }) {
        throw new GradleException("bundleRelease needs the upload key: set EMBERHUT_ANDROID_KEYSTORE_PROPERTIES (env.example)")
    }
}
`;

const releaseConfig = `
        release {
            if (emberhutSigning != null) {
                storeFile file(emberhutSigning.getProperty("storeFile"))
                storePassword emberhutSigning.getProperty("storePassword")
                keyAlias emberhutSigning.getProperty("keyAlias")
                keyPassword emberhutSigning.getProperty("keyPassword")
            }
        }`;

/**
 * Adds the loader and the release signing config to the template's android/app/build.gradle, once; throws when the
 * template no longer has what it edits (a later Expo SDK's), so a release build never silently stays debug-signed.
 * @param {string} gradle
 * @returns {string}
 */
function apply(gradle) {
  if (gradle.includes(marker)) return gradle;
  const android = /^android\s*\{/m;
  const signingConfigs = /signingConfigs\s*\{/;
  const releaseSigning = /(buildTypes\s*\{[\s\S]*?release\s*\{[\s\S]*?)signingConfig\s+signingConfigs\.debug/;
  if (!android.test(gradle) || !signingConfigs.test(gradle) || !releaseSigning.test(gradle)) {
    throw new Error("withReleaseSigning: android/app/build.gradle no longer has the template's signingConfigs and release buildType");
  }
  return gradle
    .replace(android, `${loader}\nandroid {`)
    .replace(signingConfigs, (block) => block + releaseConfig)
    .replace(releaseSigning, "$1signingConfig emberhutSigning != null ? signingConfigs.release : signingConfigs.debug");
}

/** @type {import("expo/config-plugins").ConfigPlugin} */
const withReleaseSigning = (config) =>
  withAppBuildGradle(config, (mod) => {
    mod.modResults.contents = apply(mod.modResults.contents);
    return mod;
  });

module.exports = withReleaseSigning;
module.exports.apply = apply;
module.exports.envLine = envLine;
module.exports.signingProperties = signingProperties;
