// @ts-check
// The camera as the app uses it: only the pairing QR scanner (expo-camera's barcode scanner). So on Android the camera
// is declared optional, letting the app install on a device without one (where pairing is by pasting or typing), and
// expo-camera's document scanner, which the app never uses, is left out of the build with its ML Kit library.
const { AndroidConfig, withAndroidManifest, withGradleProperties } = require("expo/config-plugins");

const cameraFeature = "android.hardware.camera";
const documentScannerProperty = "expo.camera.document-scanner-enabled";

/**
 * Declares the camera optional, once: `<uses-feature android:name="android.hardware.camera" android:required="false"/>`.
 * Without it, Android takes the CAMERA permission to mean the app needs a camera.
 * @param {AndroidConfig.Manifest.AndroidManifest} manifest
 */
function applyManifest(manifest) {
  const features = (manifest.manifest["uses-feature"] ??= []);
  const declared = features.find((feature) => feature.$["android:name"] === cameraFeature);
  if (declared) declared.$["android:required"] = "false";
  else features.push({ $: { "android:name": cameraFeature, "android:required": "false" } });
  return manifest;
}

/**
 * Turns expo-camera's document scanner off in gradle.properties.
 * @param {AndroidConfig.Properties.PropertiesItem[]} properties
 */
function applyProperties(properties) {
  return AndroidConfig.BuildProperties.updateAndroidBuildProperty(properties, documentScannerProperty, "false");
}

/** @type {import("expo/config-plugins").ConfigPlugin} */
const withScannerCamera = (config) => {
  config = withAndroidManifest(config, (mod) => {
    mod.modResults = applyManifest(mod.modResults);
    return mod;
  });
  return withGradleProperties(config, (mod) => {
    mod.modResults = applyProperties(mod.modResults);
    return mod;
  });
};

module.exports = withScannerCamera;
module.exports.applyManifest = applyManifest;
module.exports.applyProperties = applyProperties;
