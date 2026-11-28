#!/usr/bin/env -S node --disable-warning=ExperimentalWarning
// The runner's command line, run by Node's own type stripping (no build step): see src/cli.ts.
import process from "node:process";
import { main } from "../src/cli.ts";
import { messageOf } from "../src/log.ts";

main(process.argv.slice(2)).then(
  (code) => process.exit(code),
  (e) => {
    process.stderr.write(`emberhut-runner: ${messageOf(e)}\n`);
    process.exit(1);
  },
);
