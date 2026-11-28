//! Check for Updates… (the app menu): Settings' Updates page opens and a check runs now,
//! answered in a small dialog. Nothing newer: "You're on the latest (0.2.0, build 12)", with how
//! old the release's feed is. A newer build, downloaded and verified: "Emberhut 0.3.0 (build 15)
//! is ready" with its notes, Install and Restart and Later. A build already downloaded installs
//! at once. A failure says why ("Couldn't reach GitHub").

use gpui_kit::{App, Entity, PromptLevel, Window};

use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::settings::SettingsPane;
use crate::store::updates::Answer;

/// Opens Settings' Updates page in `window`, checks now, and answers there.
pub(crate) fn check_for_updates(window: &mut Window, cx: &mut App) {
    let Some(store) = crate::shell::windows::store(cx) else {
        return;
    };
    super::routes::open_settings(Some(SettingsPane::Updates), window, cx);
    let task = store.update(cx, Store::check_for_updates);
    window
        .spawn(cx, async move |cx| {
            let answer = task.await;
            if let Err(error) = cx.update(|window, cx| answer_in(&answer, &store, window, cx)) {
                log::debug!("the window closed before the update check answered: {error}");
            }
        })
        .detach();
}

/// What the dialog says for `answer`: its level, its message, its detail, its buttons.
pub(crate) fn words(
    answer: &Answer,
) -> Option<(PromptLevel, String, Option<String>, &'static [&'static str])> {
    Some(match answer {
        Answer::Installing => return None,
        Answer::Busy => (
            PromptLevel::Info,
            format!("{APP_NAME} is checking for updates"),
            Some("It says here when it knows.".to_owned()),
            &["OK"],
        ),
        Answer::Latest { running, detail } => (
            PromptLevel::Info,
            format!("You're on the latest ({running})"),
            detail.clone(),
            &["OK"],
        ),
        Answer::Ready { title, notes } => (
            PromptLevel::Info,
            title.clone(),
            (!notes.is_empty()).then(|| notes.clone()),
            &["Install and Restart", "Later"],
        ),
        Answer::Failed(why) => (
            PromptLevel::Warning,
            "Couldn't check for updates".to_owned(),
            Some(why.clone()),
            &["OK"],
        ),
        Answer::Unavailable { running, why } => (
            PromptLevel::Info,
            format!("You're on {APP_NAME} {running}"),
            Some(why.clone()),
            &["OK"],
        ),
    })
}

fn answer_in(answer: &Answer, store: &Entity<Store>, window: &mut Window, cx: &mut App) {
    let Some((level, message, detail, buttons)) = words(answer) else {
        return;
    };
    let pressed = window.prompt(level, &message, detail.as_deref(), buttons, cx);
    if !matches!(answer, Answer::Ready { .. }) {
        // One button, which only closes it: nothing to wait for.
        drop(pressed);
        return;
    }
    let store = store.clone();
    cx.spawn(async move |cx| {
        if pressed.await == Ok(0) {
            cx.update(|cx| store.update(cx, Store::restart_to_update));
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_answer_reads_as_the_reference_client_says_it() {
        let latest = Answer::Latest {
            running: "0.2.0, build 12".into(),
            detail: Some("Published 3 days ago".into()),
        };
        let (_, message, detail, buttons) = words(&latest).unwrap();
        assert_eq!(message, "You're on the latest (0.2.0, build 12)");
        assert_eq!(detail.as_deref(), Some("Published 3 days ago"));
        assert_eq!(buttons, ["OK"]);
        let ready = Answer::Ready {
            title: "Emberhut 0.3.0 (build 15) is ready".into(),
            notes: "- Faster.".into(),
        };
        let (_, message, _, buttons) = words(&ready).unwrap();
        assert_eq!(message, "Emberhut 0.3.0 (build 15) is ready");
        assert_eq!(buttons, ["Install and Restart", "Later"]);
        let failed = Answer::Failed("Couldn't reach GitHub".into());
        let (level, _, detail, _) = words(&failed).unwrap();
        assert_eq!(level, PromptLevel::Warning);
        assert_eq!(detail.as_deref(), Some("Couldn't reach GitHub"));
        let dev = Answer::Unavailable {
            running: "0.1.0, development build".into(),
            why: "Development builds don't update themselves.".into(),
        };
        let (_, message, _, _) = words(&dev).unwrap();
        assert_eq!(message, "You're on Emberhut 0.1.0, development build");
        assert_eq!(words(&Answer::Installing), None);
    }
}
