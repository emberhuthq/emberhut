//! The MCP endpoint's wire values: its path, the server's name in the `initialize` answer, and
//! the tool names agents' prompts carry.

/// The server's name in the MCP `initialize` answer, which agent configurations name the
/// server by.
pub const SERVER_NAME: &str = "emberhut";

/// The MCP endpoint's path.
pub const ROOM_PATH: &str = "/mcp/emberhut";

/// The name of the assistant's tool for the account it runs on, which agents' prompts name.
pub const ACCOUNT_TOOL: &str = "assistant_account";
