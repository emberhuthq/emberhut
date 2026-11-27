//! An X post to reply to and the replies drafted for it: an artifact of kind `x_reply` carries
//! one in its `x`. The person copies a reply, posts it on X themselves, and marks which one went
//! out (`POST /api/settings` with `{ xPosted, option }`), which closes the task.
//!
//! The keys are snake case, as the reply finder writes them. Counts come as JSON numbers that
//! may be written as floats; they read as whole numbers.

use serde::{Deserialize, Deserializer, Serialize};

use crate::time::Timestamp;

/// The post and its drafted replies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct XReply {
    /// The post's address.
    #[serde(default)]
    pub post_url: String,
    /// The post's id.
    #[serde(default)]
    pub post_id: String,
    /// Its author's handle, without the `@`.
    #[serde(default)]
    pub author: Option<String>,
    /// Its author's name.
    #[serde(default)]
    pub author_name: Option<String>,
    /// How many follow its author.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole"
    )]
    pub followers: Option<u64>,
    /// When it was posted.
    #[serde(default)]
    pub posted_at: Option<Timestamp>,
    /// Its text.
    #[serde(default)]
    pub post_text: Option<String>,
    /// The drafted replies.
    #[serde(default)]
    pub replies: Vec<XReplyOption>,
    /// What to check before posting (facts, claims about the person), if anything.
    #[serde(default)]
    pub check: Option<String>,
    /// How the post is doing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<XMetrics>,
    /// Its reply section: what wins there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<XRoom>,
    /// The reply the person posted, once they marked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted: Option<XPosted>,
}

/// One drafted reply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct XReplyOption {
    /// The reply.
    pub text: String,
    /// Why it works.
    #[serde(default)]
    pub why: Option<String>,
    /// A name for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Its style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// X's reply box with the reply typed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_url: Option<String>,
}

/// How a post is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct XMetrics {
    /// Its likes.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole"
    )]
    pub likes: Option<u64>,
    /// Its replies.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole"
    )]
    pub replies: Option<u64>,
    /// Its reposts.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole"
    )]
    pub reposts: Option<u64>,
    /// Its views.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole"
    )]
    pub views: Option<u64>,
}

/// A post's reply section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct XRoom {
    /// What the winning replies there have in common.
    #[serde(default)]
    pub winners_pattern: Option<String>,
    /// The top replies there.
    #[serde(default)]
    pub top_replies: Vec<XTopReply>,
}

/// A top reply in a post's reply section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct XTopReply {
    /// Its author's handle.
    #[serde(default)]
    pub author: String,
    /// Its text.
    #[serde(default)]
    pub text: String,
    /// Its likes.
    #[serde(default, deserialize_with = "whole")]
    pub likes: Option<u64>,
    /// Where it is.
    #[serde(default)]
    pub url: Option<String>,
}

/// The reply the person marked as posted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct XPosted {
    /// Which of the drafted replies (from 0).
    pub option: u32,
    /// What was posted.
    pub text: String,
    /// When it was marked.
    pub at: Timestamp,
}

/// A count that may come as a float (`1200.0`): read as a whole number, a negative one as 0.
fn whole<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    let number: Option<f64> = Option::deserialize(deserializer)?;
    Ok(number.map(|n| {
        let rounded = n.round().max(0.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "rounded and at least 0; a count past u64 saturates, as `as` does"
        )]
        let count = rounded as u64;
        count
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_as_whole_numbers() {
        let metrics: XMetrics =
            serde_json::from_str(r#"{"likes": 1200.0, "replies": 3, "views": -1}"#).unwrap();
        assert_eq!(metrics.likes, Some(1200));
        assert_eq!(metrics.replies, Some(3));
        assert_eq!(metrics.reposts, None);
        assert_eq!(metrics.views, Some(0));
    }
}
