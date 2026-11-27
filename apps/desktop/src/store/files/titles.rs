//! A report's own title for its card in the chat: its page read once (through the kept files,
//! quietly) when the chat shows it, and only the title held. Until it is read, or when the page
//! has none, the card shows the file's name without its extension.

use std::path::Path;

use emberhut_api::ids::RoomId;
use gpui_kit::AppContext as _;
use gpui_kit::{Context, SharedString};

use crate::config::ServerUrl;
use crate::store::reports::title_of;
use crate::store::{Store, update_or_log};

/// What a report's card is titled before (or without) its page's own title: its name without
/// the extension.
#[must_use]
pub(crate) fn stem(name: &str) -> String {
    Path::new(name).file_stem().map_or_else(
        || name.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

impl Store {
    /// The title of the report at `path`, once its page was read and has one.
    #[must_use]
    pub(crate) fn report_title(&self, path: &str) -> Option<&SharedString> {
        self.kept.titles.get(path)?.as_ref()
    }

    /// Reads the titles of the reports `room`'s held chat shows that weren't read yet.
    pub(in crate::store) fn fetch_report_titles(
        &mut self,
        room: RoomId,
        cx: &mut Context<'_, Self>,
    ) {
        let paths: Vec<String> = self
            .chats
            .messages(room)
            .iter()
            .flat_map(|message| &message.files)
            .filter(|file| file.is_html() && ServerUrl::is_path(&file.url))
            .map(|file| file.url.clone())
            .filter(|url| !self.kept.titles.contains_key(url))
            .collect();
        for path in paths {
            self.kept.titles.insert(path.clone(), None);
            let bytes = self.file_bytes(&path, cx);
            cx.spawn(async move |this, cx| {
                let Ok(bytes) = bytes.await else {
                    return;
                };
                let title = cx
                    .background_spawn(async move { title_of(&String::from_utf8_lossy(&bytes)) })
                    .await;
                update_or_log(&this, cx, |store, cx| {
                    store
                        .kept
                        .titles
                        .insert(path, title.map(SharedString::from));
                    cx.notify();
                });
            })
            .detach();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_without_its_title_shows_the_name_without_its_extension() {
        assert_eq!(stem("weekly-report.html"), "weekly-report");
        assert_eq!(stem("notes"), "notes");
    }
}
