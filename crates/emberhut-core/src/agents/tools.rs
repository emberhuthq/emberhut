//! An agent's tools: some of the server's own MCP tools, and MCP servers of its own.
//!
//! An agent other than the default gets only what its list names. The server's tools that act
//! as the owner (send in their name, drive their browser or Mac, read their messages, mail,
//! health or calendar) are never in that list: [`acts_as_owner`] is the rule, checked when the
//! list is written and again when the agent's tool set is built.

use std::collections::HashSet;

use super::error::AgentError;
use super::id::AgentId;
use crate::wire::wire_enum;

/// The longest tool or MCP server name, in bytes.
const NAME_MAX: usize = 64;
/// The longest MCP configuration path, in bytes.
const CONFIG_MAX: usize = 1024;

/// The server's tools that act as the owner, by exact name.
pub const OWNER_TOOLS: &[&str] = &[
    "health",
    "screen_time",
    "calendar",
    "reminders",
    "add_reminder",
    "add_event",
    "replied",
];

/// The families of the server's tools that act as the owner, by name prefix: the browser's and
/// Chrome's, the Mac's, the owner's messages, mail, Slack and WhatsApp.
pub const OWNER_TOOL_PREFIXES: &[&str] = &[
    "browser_",
    "chrome_",
    "mac_",
    "messages_",
    "gmail_",
    "slack_",
    "wa_",
    "whatsapp_",
];

/// Whether the server's tool `name` acts as the owner, so only the default agent may have it.
///
/// ```
/// use emberhut_core::agents::acts_as_owner;
///
/// assert!(acts_as_owner("slack_send"));
/// assert!(acts_as_owner("browser_click"));
/// assert!(!acts_as_owner("tasks"));
/// ```
#[must_use]
pub fn acts_as_owner(name: &str) -> bool {
    OWNER_TOOLS.contains(&name) || OWNER_TOOL_PREFIXES.iter().any(|p| name.starts_with(p))
}

wire_enum! {
    /// Where a tool comes from.
    pub enum ToolKind {
        /// One of the server's own MCP tools.
        Server = "server",
        /// An MCP server of the agent's own.
        Mcp = "mcp",
    }
}

/// One tool an agent has.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AgentTool {
    /// One of the server's own MCP tools, by its name.
    Server {
        /// The tool's name (`tasks`, `facts`).
        name: String,
    },
    /// An MCP server of the agent's own.
    Mcp {
        /// Its name: its tools are `mcp__<name>__*`.
        name: String,
        /// The absolute path of a JSON file, outside the repository, giving the server's command
        /// or URL.
        config: String,
    },
}

impl AgentTool {
    /// Where it comes from.
    #[must_use]
    pub fn kind(&self) -> ToolKind {
        match self {
            Self::Server { .. } => ToolKind::Server,
            Self::Mcp { .. } => ToolKind::Mcp,
        }
    }

    /// Its name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Server { name } | Self::Mcp { name, .. } => name,
        }
    }

    /// What its session's `--allowedTools` lists for it: the tool's name, or `mcp__<name>__*`
    /// for every tool of an MCP server.
    #[must_use]
    pub fn allowed_tools(&self) -> String {
        match self {
            Self::Server { name } => name.clone(),
            Self::Mcp { name, .. } => format!("mcp__{name}__*"),
        }
    }

    /// Checks the tool's own shape: a server tool's name is `a-z`, `0-9` and `_`; an MCP
    /// server's is `a-z`, `0-9`, `-` and single `_`s (a double one would end its tools' prefix),
    /// and its configuration an absolute path to a `.json` file.
    ///
    /// # Errors
    ///
    /// Why not, in words.
    pub fn check(&self) -> Result<(), &'static str> {
        match self {
            Self::Server { name } => {
                let fits = !name.is_empty()
                    && name.len() <= NAME_MAX
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
                if !fits {
                    return Err("a server tool's name is a-z, 0-9 and _");
                }
            }
            Self::Mcp { name, config } => {
                let fits = name
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    && name.len() <= NAME_MAX
                    && !name.contains("__")
                    && name.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
                    });
                if !fits {
                    return Err(
                        "an MCP server's name is a-z, 0-9, '-' and single '_', starting with a \
                         letter or digit",
                    );
                }
                let path_fits = config.starts_with('/')
                    && config.len() <= CONFIG_MAX
                    && std::path::Path::new(config)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
                    && !config.split('/').any(|part| part == "..")
                    && !config.chars().any(char::is_control);
                if !path_fits {
                    return Err("an MCP server's config is the absolute path of a .json file");
                }
            }
        }
        Ok(())
    }
}

/// Checks `agent`'s whole list of tools: each one's shape, none twice, and none that acts as
/// the owner. The default agent has every tool already, so it takes no list at all.
///
/// # Errors
///
/// [`AgentError::DefaultAgent`] for the default agent, [`AgentError::OwnerTool`] or
/// [`AgentError::Tool`] naming the first tool refused.
pub fn validate_tools(agent: &AgentId, tools: &[AgentTool]) -> Result<(), AgentError> {
    if agent.is_main() {
        return Err(AgentError::DefaultAgent(
            "has every tool, and its tool set is not narrowed",
        ));
    }
    let mut seen = HashSet::new();
    for (index, tool) in tools.iter().enumerate() {
        tool.check()
            .map_err(|why| AgentError::Tool { index, why })?;
        if let AgentTool::Server { name } = tool
            && acts_as_owner(name)
        {
            return Err(AgentError::OwnerTool {
                index,
                name: name.clone(),
            });
        }
        if !seen.insert((tool.kind(), tool.name())) {
            return Err(AgentError::Tool {
                index,
                why: "listed twice",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> AgentId {
        AgentId::parse("client-a").unwrap()
    }

    fn server(name: &str) -> AgentTool {
        AgentTool::Server {
            name: name.to_owned(),
        }
    }

    fn mcp(name: &str, config: &str) -> AgentTool {
        AgentTool::Mcp {
            name: name.to_owned(),
            config: config.to_owned(),
        }
    }

    #[test]
    fn the_owners_tools_act_as_the_owner() {
        for name in [
            "slack_send",
            "slack_file",
            "whatsapp_send",
            "wa_group_reply",
            "browser_open",
            "chrome_pair",
            "mac_app_click",
            "mac_record",
            "messages_read",
            "gmail_connect",
            "health",
            "screen_time",
            "calendar",
            "add_event",
            "replied",
        ] {
            assert!(acts_as_owner(name), "{name}");
        }
        for name in [
            "tasks",
            "facts",
            "recall",
            "task_note",
            "send_file",
            "theme",
        ] {
            assert!(!acts_as_owner(name), "{name}");
        }
    }

    #[test]
    fn a_good_list() {
        let tools = [
            server("tasks"),
            server("facts"),
            mcp("example-admin", "/srv/agents/example-admin.json"),
        ];
        assert_eq!(validate_tools(&agent(), &tools), Ok(()));
        assert_eq!(tools[2].allowed_tools(), "mcp__example-admin__*");
        assert_eq!(tools[0].allowed_tools(), "tasks");
        assert_eq!(tools[2].kind(), ToolKind::Mcp);
    }

    #[test]
    fn an_owner_tool_is_refused_for_another_agent() {
        assert_eq!(
            validate_tools(&agent(), &[server("tasks"), server("slack_send")]),
            Err(AgentError::OwnerTool {
                index: 1,
                name: "slack_send".to_owned()
            })
        );
    }

    #[test]
    fn the_default_agent_is_not_narrowed() {
        assert!(matches!(
            validate_tools(&AgentId::MAIN, &[server("tasks")]),
            Err(AgentError::DefaultAgent(_))
        ));
    }

    #[test]
    fn bad_shapes_are_refused() {
        for tool in [
            server(""),
            server("Tasks"),
            mcp("a__b", "/srv/a.json"),
            mcp("_a", "/srv/a.json"),
            mcp("a", "relative.json"),
            mcp("a", "/srv/a.toml"),
            mcp("a", "/srv/../etc/a.json"),
        ] {
            assert!(
                matches!(
                    validate_tools(&agent(), std::slice::from_ref(&tool)),
                    Err(AgentError::Tool { index: 0, .. })
                ),
                "{tool:?}"
            );
        }
    }

    #[test]
    fn a_tool_listed_twice_is_refused() {
        assert_eq!(
            validate_tools(
                &agent(),
                &[mcp("a", "/srv/a.json"), mcp("a", "/srv/b.json")]
            ),
            Err(AgentError::Tool {
                index: 1,
                why: "listed twice"
            })
        );
    }
}
