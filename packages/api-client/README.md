# @emberhut/api-client

The TypeScript client for the emberhut API, used by the mobile and web apps. Placeholder: it
arrives after the server's first slice.

It will be generated, never edited: `openapi-typescript` builds the types from
`crates/emberhut-server/openapi.json` (the document the Rust server produces), and `openapi-fetch`
is the runtime. A CI job will regenerate it and fail if the committed client differs. See
[docs/architecture.md](../../docs/architecture.md).
