/** A pairing code is 8 letters and digits, shown as ABCD-EFGH (the server's format). */
export const codeLength = 8;
const groupLength = 4;

/**
 * What you typed, as the code field shows it: capitals, letters and digits only, the dash put back after the fourth.
 * Pasting "abcd efgh" or "ABCD-EFGH" gives the same code; anything past eight characters is dropped.
 */
export function formatCode(typed: string): string {
  const plain = typed
    .toUpperCase()
    .replace(/[^A-Z0-9]/g, "")
    .slice(0, codeLength);
  return plain.length > groupLength ? `${plain.slice(0, groupLength)}-${plain.slice(groupLength)}` : plain;
}

/** All eight characters are there. */
export function isComplete(code: string): boolean {
  return code.replace(/-/g, "").length === codeLength;
}
