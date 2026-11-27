import AsyncStorage from "@react-native-async-storage/async-storage";
import * as Crypto from "expo-crypto";
import { useEffect, useState } from "react";
import { inBackground } from "@/log";

const key = "emberhut.browserDevice";
let known: Promise<string> | null = null;

/** This device's name for Emberhut's browser: who took it over, so another of your devices can't drive at the same time. */
function browserDevice(): Promise<string> {
  known ??= AsyncStorage.getItem(key).then(async (held) => {
    if (held) return held;
    const id = Crypto.randomUUID();
    await AsyncStorage.setItem(key, id);
    return id;
  });
  return known;
}

/** The same, for a screen: null until it's read. */
export function useBrowserDevice(): string | null {
  const [device, setDevice] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    inBackground(
      browserDevice().then((id) => live && setDevice(id)),
      "browserDevice: browserDevice",
    );
    return () => {
      live = false;
    };
  }, []);
  return device;
}
