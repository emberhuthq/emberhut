//! An X post's drafted replies: Reply on X puts the reply on the clipboard and opens X's reply
//! box with it typed in (the reply finder's own link, else one made from the post), so posting
//! it is one paste; Posted tells the server which reply went out, which closes the task.

use std::sync::Arc;

use emberhut_api::ids::ArtifactId;
use emberhut_api::query::encoded;
use emberhut_api::settings::SettingsAction;
use emberhut_api::x_reply::{XReply, XReplyOption};
use gpui_kit::{AppContext as _, Context};

use super::{Store, update_or_log};

/// X's reply box for `option` on `post`: the finder's link when it made one and it is a web
/// address (`https`), else X's intent link with the post and the reply in it. Either is still
/// opened through the app's link rules ([`crate::config::ServerUrl::link`]).
#[must_use]
pub(crate) fn intent_url(post: &XReply, option: &XReplyOption) -> String {
    let finders = option.intent_url.as_deref().filter(|url| {
        url.get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    });
    finders.map_or_else(
        || {
            let mut url = String::from("https://x.com/intent/tweet?in_reply_to=");
            url.push_str(&encoded(&post.post_id));
            url.push_str("&text=");
            url.push_str(&encoded(&option.text));
            url
        },
        str::to_owned,
    )
}

/// "@example · 1.2K followers · 3 hours ago · 40 likes": who a post reaches and how it does.
#[must_use]
pub(crate) fn post_facts(post: &XReply, now: jiff::Timestamp) -> String {
    let age = post
        .posted_at
        .as_ref()
        .and_then(super::tasks::clock::instant)
        .map(|at| super::tasks::clock::relative(at, now));
    let metrics = post.metrics.unwrap_or_default();
    [
        post.followers.map(|n| format!("{} followers", compact(n))),
        age,
        metrics.likes.map(|n| format!("{} likes", compact(n))),
        metrics.replies.map(|n| format!("{} replies", compact(n))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// A count as it reads small: `950`, `1.2K`, `3.4M`.
#[must_use]
pub(crate) fn compact(n: u64) -> String {
    let short = |n: u64, unit: u64, suffix: &str| {
        let tenths = n * 10 / unit;
        if tenths.is_multiple_of(10) || tenths >= 100 {
            format!("{}{suffix}", tenths / 10)
        } else {
            format!("{}.{}{suffix}", tenths / 10, tenths % 10)
        }
    };
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => short(n, 1_000, "K"),
        _ => short(n, 1_000_000, "M"),
    }
}

impl Store {
    /// The user posted drafted reply `option` (from 0) of `artifact` on X: the server keeps it
    /// and closes the task.
    pub(crate) fn mark_posted(
        &mut self,
        artifact: ArtifactId,
        option: u32,
        cx: &mut Context<'_, Self>,
    ) {
        let (backend, kicker) = (Arc::clone(&self.backend), self.kicker.clone());
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_spawn(async move {
                    backend.settings(SettingsAction::XPosted { artifact, option })
                })
                .await;
            kicker.kick();
            update_or_log(&this, cx, |store, cx| {
                if let Err(error) = done {
                    store.action_failed("Not marked posted", &error, cx);
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> XReply {
        XReply {
            post_id: "123".to_owned(),
            followers: Some(1_234),
            metrics: Some(emberhut_api::x_reply::XMetrics {
                likes: Some(40),
                ..emberhut_api::x_reply::XMetrics::default()
            }),
            posted_at: Some(emberhut_api::time::Timestamp::new("2026-11-25T09:00:00Z")),
            ..XReply::default()
        }
    }

    #[test]
    fn the_reply_box_is_the_finders_link_or_one_made_from_the_post() {
        let made = XReplyOption {
            text: "Nice one & thanks!".to_owned(),
            ..XReplyOption::default()
        };
        assert_eq!(
            intent_url(&post(), &made),
            "https://x.com/intent/tweet?in_reply_to=123&text=Nice%20one%20%26%20thanks%21"
        );
        let own = XReplyOption {
            intent_url: Some("https://x.com/intent/tweet?x=1".to_owned()),
            ..made
        };
        assert_eq!(intent_url(&post(), &own), "https://x.com/intent/tweet?x=1");
        for refused in [
            "file:///Applications/Calculator.app",
            "smb://example.com/x",
            "x-app://",
        ] {
            let shut = XReplyOption {
                intent_url: Some(refused.to_owned()),
                ..own.clone()
            };
            assert_eq!(
                intent_url(&post(), &shut),
                "https://x.com/intent/tweet?in_reply_to=123&text=Nice%20one%20%26%20thanks%21",
                "{refused}: the link made from the post instead"
            );
        }
    }

    #[test]
    fn a_post_says_who_it_reaches_and_how_it_does() {
        let now = "2026-11-25T12:00:00Z".parse().unwrap();
        assert_eq!(
            post_facts(&post(), now),
            "1.2K followers · 3 hours ago · 40 likes"
        );
        assert_eq!(compact(950), "950");
        assert_eq!(compact(12_000), "12K");
        assert_eq!(compact(3_450_000), "3.4M");
    }
}
