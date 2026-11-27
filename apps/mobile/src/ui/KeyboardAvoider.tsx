import type { ReactNode } from "react";
import type { StyleProp, ViewStyle } from "react-native";
import { KeyboardAvoidingView } from "react-native-keyboard-controller";

export interface KeyboardAvoiderProps {
  children: ReactNode;
  style?: StyleProp<ViewStyle>;
}

/**
 * Keeps its content above the keyboard: its bottom is padded by however much of it the keyboard covers, frame by frame
 * as the keyboard moves (react-native-keyboard-controller reads it natively). It finds its own place on the screen, so a
 * header above or the tab bar below needs no offset. The chat has its own, for interactive dismiss (ChatScreen).
 */
export function KeyboardAvoider({ children, style }: KeyboardAvoiderProps) {
  return (
    <KeyboardAvoidingView behavior="padding" automaticOffset style={[{ flex: 1 }, style]}>
      {children}
    </KeyboardAvoidingView>
  );
}
