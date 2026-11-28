//! The front door's Suggested rows: a handful of plain verbs worked out from how things stand
//! ([`Now`]), what needs the user first. Each opens a setting of [`super::entries`] (its page,
//! or its row); none changes anything by itself, and none offers what is already done.

use gpui_kit::assets::IconName;

use super::entries::{Entry, entry};
use super::now::{Now, computer};

/// The most rows Suggested shows.
pub(crate) const MOST: usize = 7;

/// A suggested row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suggestion {
    /// The setting it opens.
    pub(crate) entry: &'static Entry,
    /// Its verb: `Review 3 lessons`.
    pub(crate) title: String,
    /// Its one line.
    pub(crate) line: String,
    /// Its icon: its setting's, or one for what it does (a sun for Switch to light).
    pub(crate) icon: IconName,
}

/// `count` of `one`, said as people do: `1 lesson`, `3 lessons`.
fn counted(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

/// A suggestion's icon: its setting's, but the sun for Switch to light (the setting's is a
/// moon).
fn icon(title: &str, entry: &Entry) -> IconName {
    if title == "Switch to light" {
        IconName::Sun
    } else {
        entry.icon
    }
}

/// What needs a look, first in line: lessons to review, what is broken, a build ready to
/// install.
fn needs_a_look(now: &Now) -> Vec<(&'static str, String, String)> {
    let mut rows = Vec::new();
    if now.lessons > 0 {
        rows.push((
            "lessons",
            format!("Review {}", counted(now.lessons, "lesson")),
            "Your ember is trying them out: keep or drop each".to_owned(),
        ));
    }
    if let Some(profile) = now.chrome_offline.first() {
        rows.push((
            "chrome",
            format!("Fix Chrome's {profile} profile"),
            "It isn't online: pair it again or hand it back".to_owned(),
        ));
    }
    if let Some(tool) = now.failing_tools.first() {
        rows.push((
            "tools",
            format!("Reconnect {tool}"),
            "It can't connect, so your ember can't use it".to_owned(),
        ));
    }
    if let Some(source) = now.sources_to_fix.first() {
        rows.push((
            "sources",
            format!("Fix {source}"),
            "It needs a look before your ember can read it".to_owned(),
        ));
    }
    if let Some(version) = &now.update_ready {
        rows.push((
            "updates",
            format!("Install {} {version} and restart", crate::brand::APP_NAME),
            "Downloaded and checked: it reopens in a few seconds".to_owned(),
        ));
    }
    rows
}

/// What to suggest now, what needs a look first, at most [`MOST`].
#[must_use]
pub(crate) fn suggestions(now: &Now) -> Vec<Suggestion> {
    let mut rows = needs_a_look(now);
    if now.signed_in {
        if now.phone_paired {
            rows.push((
                "pair",
                "Add a phone or computer".to_owned(),
                "One code, works once for 10 minutes".to_owned(),
            ));
        } else {
            rows.push((
                "pair",
                "Pair your phone".to_owned(),
                "One QR code, works once for 10 minutes".to_owned(),
            ));
        }
    }
    rows.push((
        "thinks-with",
        "Change what your ember thinks with".to_owned(),
        now.provider
            .clone()
            .unwrap_or_else(|| "Claude, OpenAI, DeepSeek, OpenRouter, Ollama…".to_owned()),
    ));
    let now_word = match now.appearance_word().as_str() {
        word @ ("Light" | "Dark") => word.to_lowercase(),
        _ => format!("as {} is", computer()),
    };
    rows.push((
        "appearance-mode",
        if now.dark {
            "Switch to light".to_owned()
        } else {
            "Switch to dark".to_owned()
        },
        format!("Now: {now_word}"),
    ));
    if now.on_duty {
        rows.push((
            "on-duty",
            "Go off duty".to_owned(),
            "Stops working on its own. Still answers when you message it.".to_owned(),
        ));
    } else {
        rows.push((
            "on-duty",
            "Go on duty".to_owned(),
            "Starts working on its own again".to_owned(),
        ));
    }
    rows.push((
        "about-you",
        "See what your ember knows about you".to_owned(),
        match now.facts {
            Some(count) => format!("{} · fix or forget any", counted(count, "thing")),
            None => "Fix or forget any of it".to_owned(),
        },
    ));
    rows.into_iter()
        .take(MOST)
        .filter_map(|(id, title, line)| {
            let entry = entry(id)?;
            let icon = icon(&title, entry);
            Some(Suggestion {
                entry,
                title,
                line,
                icon,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(now: &Now) -> Vec<String> {
        suggestions(now).into_iter().map(|row| row.title).collect()
    }

    #[test]
    fn lessons_waiting_come_first() {
        let now = Now {
            lessons: 3,
            ..Now::default()
        };
        let rows = suggestions(&now);
        assert_eq!(rows[0].title, "Review 3 lessons");
        assert_eq!(rows[0].entry.id, "lessons");
        assert!(
            !titles(&Now::default())
                .iter()
                .any(|t| t.starts_with("Review"))
        );
    }

    #[test]
    fn no_phone_suggests_pairing_one_and_a_phone_suggests_another_device() {
        let alone = Now {
            signed_in: true,
            ..Now::default()
        };
        assert!(titles(&alone).contains(&"Pair your phone".to_owned()));
        let paired = Now {
            phone_paired: true,
            ..alone
        };
        let shown = titles(&paired);
        assert!(!shown.contains(&"Pair your phone".to_owned()));
        assert!(shown.contains(&"Add a phone or computer".to_owned()));
    }

    #[test]
    fn what_needs_a_look_is_suggested_and_what_is_fine_is_not() {
        let fine = Now {
            sources_on: 1,
            ..Now::default()
        };
        assert!(
            !titles(&fine)
                .iter()
                .any(|t| t.starts_with("Fix") || t.starts_with("Reconnect")),
            "Gmail on: nothing to connect or fix"
        );
        let broken = Now {
            chrome_offline: vec!["Work".into()],
            failing_tools: vec!["linear".into()],
            sources_to_fix: vec!["Gmail".into()],
            ..Now::default()
        };
        let shown = titles(&broken);
        assert_eq!(
            shown[..3],
            ["Fix Chrome's Work profile", "Reconnect linear", "Fix Gmail"]
        );
    }

    #[test]
    fn the_opposite_of_what_shows_is_offered() {
        let dark = Now {
            dark: true,
            on_duty: true,
            ..Now::default()
        };
        let shown = titles(&dark);
        assert!(shown.contains(&"Switch to light".to_owned()));
        let light = suggestions(&dark)
            .into_iter()
            .find(|row| row.title == "Switch to light")
            .unwrap();
        assert_eq!(light.icon, IconName::Sun, "the sun for light, not the moon");
        assert!(shown.contains(&"Go off duty".to_owned()));
        let light = titles(&Now::default());
        assert!(light.contains(&"Switch to dark".to_owned()));
        assert!(light.contains(&"Go on duty".to_owned()));
    }

    #[test]
    fn thinks_with_is_always_there_with_the_provider() {
        let now = Now {
            provider: Some("DeepSeek · key ending 0000".into()),
            ..Now::default()
        };
        let row = suggestions(&now)
            .into_iter()
            .find(|row| row.entry.id == "thinks-with")
            .unwrap();
        assert_eq!(row.line, "DeepSeek · key ending 0000");
    }

    #[test]
    fn a_ready_update_is_offered_after_what_needs_fixing() {
        assert!(
            !titles(&Now::default())
                .iter()
                .any(|t| t.starts_with("Install")),
            "nothing ready, nothing to install"
        );
        let ready = Now {
            update_ready: Some("0.2.0".into()),
            sources_to_fix: vec!["Gmail".into()],
            signed_in: true,
            ..Now::default()
        };
        let rows = suggestions(&ready);
        assert_eq!(rows[0].title, "Fix Gmail");
        assert_eq!(rows[1].title, "Install Emberhut 0.2.0 and restart");
        assert_eq!(rows[1].entry.id, "updates");
        assert_eq!(rows[2].title, "Pair your phone");
    }

    #[test]
    fn never_more_than_seven() {
        let busy = Now {
            lessons: 2,
            chrome_offline: vec!["Work".into()],
            failing_tools: vec!["linear".into()],
            sources_to_fix: vec!["Gmail".into()],
            signed_in: true,
            ..Now::default()
        };
        assert_eq!(suggestions(&busy).len(), MOST);
        assert!((4..=MOST).contains(&suggestions(&Now::default()).len()));
    }
}
