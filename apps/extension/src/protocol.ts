// The extension's side of the server's socket (`/api/chrome/ws`, emberhut_api::chrome::extension): its first message
// (`pair` or `hello`) names PROTOCOL's version. Besides the named messages below, `pair`, `hello`, `ping`, `pong`,
// `extension.initialized` and the relayed `chrome.*` calls and events are the protocol's words.

/** The protocol version the first message names. */
export const PROTOCOL = 1;

/** The named messages, both ways. */
export const Msg = {
  /** Server → extension: start working (let go of everything, report its tabs, say initialized). */
  start: "emberhut.start",
  /** Server → extension, after its hello: its name, label and the server's latest version. */
  hello: "emberhut.hello",
  /** Server → extension: paired, with its token. */
  paired: "emberhut.paired",
  /** Server → extension: the owner paused it from the app (`why`, `minutes`). */
  pause: "emberhut.pause",
  /** Server → extension: the pause is over (`why`). */
  resume: "emberhut.resume",
  /** Server → extension: its new name (`name`, `label`). */
  name: "emberhut.name",
  /** Server → extension: a step the assistant took (`text`), for the popup. */
  step: "emberhut.step",
  /** Server → extension: the assistant handed the browser to the owner (`reason`). */
  handoff: "emberhut.handoff",
  /** Both ways: the owner's turn is over (server → extension: `by`, `reason` when the assistant took it back). */
  handback: "emberhut.handback",
  /** Server → extension: the extension's folder holds `version` now. */
  update: "emberhut.update",
  /** Extension → server: the owner's pause (`why`, `until`, `turn`), or `null` when it's over. */
  stopped: "emberhut.stopped",
  /** Extension → server: the owner unpaired this profile. */
  unpair: "emberhut.unpair",
  /** Extension → server: Chrome cut the link to a tab that is still open (`tabId`, `url`). */
  cut: "emberhut.cut",
  /** Extension → server: a tab went to a blocked site and was let go of (`tabId`, `host`). */
  blocked: "emberhut.blocked",
  /** Extension → server: another extension's frame keeps it out of a tab for now (`tabId`, `url`). */
  framed: "emberhut.framed",
} as const;

/** Extension → server: ready after a start. */
export const INITIALIZED = "extension.initialized";

/** Who closed a tab, or took the browser back: the assistant. */
export const BY_ASSISTANT = "assistant";
