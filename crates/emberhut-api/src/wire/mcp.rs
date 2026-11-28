//! The MCP endpoint's wire values: its path, the server's name in the `initialize` answer, and
//! the tool names agents' prompts carry.

/// The server's name in the MCP `initialize` answer, which agent configurations name the
/// server by.
pub const SERVER_NAME: &str = "emberhut";

/// The MCP endpoint's path.
pub const ROOM_PATH: &str = "/mcp/emberhut";

/// The request header a runner session sends with its calls to the MCP endpoint: its own
/// session id, so the server knows which session (and so which task) a call is from. Runner
/// sessions share one room token; without the header a call is the room's.
pub const SESSION_HEADER: &str = "x-emberhut-session";

/// The value the server lists for [`SESSION_HEADER`] in the runners' shared MCP list: the
/// variable the runner sets to each session's id, which the agent CLI fills in when it reads
/// its MCP configuration. A value still in this form (the variable unset) names no session.
pub const SESSION_HEADER_VALUE: &str = "${EMBERHUT_SESSION}";

/// The name of the assistant's tool for the account it runs on, which agents' prompts name.
pub const ACCOUNT_TOOL: &str = "assistant_account";
