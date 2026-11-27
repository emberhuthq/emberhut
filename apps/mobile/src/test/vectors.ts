import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The connect contract's shared test vectors (crates/emberhut-api/tests/fixtures/connect/), which the Rust crates run too:
// the phone's ports of the same rules read the same files.
const dir = new URL("../../../../crates/emberhut-api/tests/fixtures/connect/", import.meta.url);

/** One vector file, parsed; the test says what shape it reads. */
export function connectVectors<T>(name: string): T {
  return JSON.parse(readFileSync(fileURLToPath(new URL(name, dir)), "utf8")) as T;
}
