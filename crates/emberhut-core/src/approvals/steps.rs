//! Which kind of approval a step the guard rails hold is, for a task made in an agent's room:
//! a final step in a page (a post or a comment, or another final click) or an outward command
//! (a merge, or a deploy). A hard step (money, a password, account security) is no kind: it
//! stays the owner's card, whatever the agent's policies say.

use super::ActionKind;
use crate::guard::grants::StepKind;

/// The kind of approval a final step of `kind` asks for; none for a hard step, which only the
/// owner OKs.
#[must_use]
pub const fn final_step(kind: StepKind) -> Option<ActionKind> {
    match kind {
        StepKind::Said => Some(ActionKind::Post),
        StepKind::Ordinary => Some(ActionKind::Other),
        StepKind::Hard => None,
    }
}

/// The kind of approval an outward command asks for, from its steps' words (as
/// `guard::outward::steps` names them): a merge when every step takes code to a repository
/// (`git …`, `gh pr …`), else a deploy (a publish, a release, a deploy, a service restart, an
/// HTTP write, or a step that can't be read).
#[must_use]
pub fn outward<S: AsRef<str>>(steps: Option<&[S]>) -> ActionKind {
    let to_a_repository = |what: &str| what.starts_with("git ") || what.starts_with("gh pr ");
    match steps {
        Some(steps) if steps.iter().all(|what| to_a_repository(what.as_ref())) => ActionKind::Merge,
        _ => ActionKind::Deploy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_final_step_is_a_post_or_another_action_and_a_hard_one_is_the_owners() {
        assert_eq!(final_step(StepKind::Said), Some(ActionKind::Post));
        assert_eq!(final_step(StepKind::Ordinary), Some(ActionKind::Other));
        assert_eq!(final_step(StepKind::Hard), None);
    }

    #[test]
    fn code_to_a_repository_is_a_merge_and_the_rest_a_deploy() {
        assert_eq!(
            outward(Some(&["git push", "gh pr merge"][..])),
            ActionKind::Merge
        );
        assert_eq!(
            outward(Some(&["git push", "npm publish"][..])),
            ActionKind::Deploy
        );
        assert_eq!(
            outward(Some(&["systemctl restart"][..])),
            ActionKind::Deploy
        );
        assert_eq!(outward::<&str>(None), ActionKind::Deploy);
    }
}
