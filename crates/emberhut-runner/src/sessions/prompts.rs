//! What the runner says to the agent: the system prompt's addition and its own short messages.
//!
//! The addition says who started the session, how to ask the owner something and where files
//! sent to it land. Each installation may replace the first part and add a note about its
//! machine (`config.json`'s `instructions`).

use std::path::Path;

use crate::config::Instructions;

/// For a session the assistant opens for work, unless the settings say otherwise.
pub const WORK: &str = "This session was started by the Emberhut assistant on this runner. Its owner can read it in the \
Emberhut app and reply here. Work as you would in the owner's own coding session, with full access.";

/// For the owner's own session, unless the settings say otherwise.
pub const DIRECT: &str = "This is the owner's own coding session, opened and driven from the Emberhut app, like a terminal \
session. Work with them as you would there, with full access. The Emberhut assistant can read this session and sometimes \
writes in it for the owner: such messages start with \"(From the assistant\"; treat them as passed on from the owner. To ask \
something with choices, use AskUserQuestion: it shows in the app and the session waits for the answer.";

/// What an owner's session is told after it asks a question: the card is out, the answer comes
/// as the next message.
pub const ASKED: &str = "Your question is in front of the owner now, in the Emberhut app, with its options. End your turn \
here with a short line; the answer comes as the next message.";

/// Carry on after the session moved to another account mid-turn.
pub const MOVED: &str = "(The runner: the account this session ran on hit its usage limit, so the session moved to another \
account. Nothing is lost. Carry on exactly where you stopped, with the same task, and end your reply as asked before.)";

/// Carry on after the runner restarted mid-turn.
pub const RESTARTED: &str = "(The runner restarted while you were working on this turn, so the command or step you were in \
was cut off before it finished. Check where things stand, run again what didn't finish, and carry on with the same task; \
end your reply as asked before.)";

/// Carry on after a restart stopped the background jobs the session waited on.
pub const JOBS_LOST: &str = "(The runner restarted, which stopped the background jobs this session was waiting on. Check \
what they were doing, run again what didn't finish, and carry on with the same task; end your reply as asked before.)";

/// Carry on once an account is free again after every one was out.
pub const FREE_AGAIN: &str =
    "(The runner: an account is free again. Carry on where you stopped, with the same task.)";

/// Where files sent to the session land.
#[must_use]
pub fn files_note(inbox: &Path) -> String {
    format!(
        "Files sent to you (designs, spreadsheets, screenshots) land in {}; you're told when one arrives.",
        inbox.display()
    )
}

/// A session in a worktree outside its project's folder is told to read the project's
/// instructions there (the agent loads only those of the folder it starts in and above).
#[must_use]
pub fn project_note(project: &Path, files: &[String]) -> String {
    format!(
        "This session works in a worktree of {}. Before anything else, read the project's instructions there ({}): \
they apply here as if you had started in that folder.",
        project.display(),
        files.join(", ")
    )
}

/// The text appended to the agent's own system prompt for a session: the session's kind (the
/// settings' text or the default), the machine's note, where files land and the project note,
/// each after a blank line.
#[must_use]
pub fn system_append(
    ins: &Instructions,
    direct: bool,
    inbox: &Path,
    project: Option<&str>,
) -> String {
    let kind = if direct {
        ins.direct.as_deref().unwrap_or(DIRECT)
    } else {
        ins.work.as_deref().unwrap_or(WORK)
    };
    let files = files_note(inbox);
    let parts = [
        Some(""),
        Some(kind),
        ins.machine.as_deref(),
        Some(files.as_str()),
        project,
    ];
    parts.into_iter().flatten().collect::<Vec<_>>().join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_replace_the_kinds_text_and_add_the_machines() {
        let ins = Instructions {
            work: Some("Work note.".to_owned()),
            direct: None,
            machine: Some("Machine note.".to_owned()),
        };
        let inbox = Path::new("/runner/inbox/r-00000001");
        let work = system_append(&ins, false, inbox, None);
        assert_eq!(
            work,
            format!("\n\nWork note.\n\nMachine note.\n\n{}", files_note(inbox))
        );
        let direct = system_append(&ins, true, inbox, Some("Project note."));
        assert!(direct.starts_with(&format!("\n\n{DIRECT}\n\nMachine note.")));
        assert!(direct.contains("/runner/inbox/r-00000001"));
        assert!(direct.ends_with("\n\nProject note."));
    }

    #[test]
    fn without_settings_the_defaults_say_who_started_the_session() {
        let ins = Instructions::default();
        let inbox = Path::new("/i");
        assert_eq!(
            system_append(&ins, false, inbox, None),
            format!("\n\n{WORK}\n\n{}", files_note(inbox))
        );
        assert!(system_append(&ins, true, inbox, None).contains("AskUserQuestion"));
    }

    #[test]
    fn the_project_note_names_the_project_and_its_files() {
        let note = project_note(
            Path::new("/w/app"),
            &["/w/app/CLAUDE.md".to_owned(), "/w/CLAUDE.md".to_owned()],
        );
        assert!(note.starts_with("This session works in a worktree of /w/app."));
        assert!(note.contains("(/w/app/CLAUDE.md, /w/CLAUDE.md)"));
    }
}
