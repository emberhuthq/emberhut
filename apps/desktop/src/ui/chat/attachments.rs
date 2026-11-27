//! The composer's attachment chips: a picture as its thumbnail, a video as its first still
//! frame with a play mark, any other file as its icon, name and size; a ring and a bar while it uploads, the reason and a retry once it failed, and
//! an X to take it out. Also how a file looks wherever the chat shows one: its icon and size.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::attachment::{
    Attachment as Chip, AttachmentContent, AttachmentDescription, AttachmentGroup, AttachmentMedia,
    AttachmentStatus, AttachmentTitle,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, ElementId, Entity, Image, ImageSource, px, white};

use crate::backend::UploadSource;
use crate::store::Store;
use crate::store::attachments::{Attachment, Stage};
use crate::ui::palette::Pal;

/// Extensions of pictures this app can draw (GPUI's decoders), shown as thumbnails.
const DRAWN: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];

/// The lower-case extension of a file name.
fn extension(name: &str) -> String {
    Path::new(name).extension().map_or_else(String::new, |ext| {
        ext.to_string_lossy().to_ascii_lowercase()
    })
}

/// Whether a file of this name is a picture this app can draw.
#[must_use]
pub(crate) fn is_drawn(name: &str) -> bool {
    DRAWN.contains(&extension(name).as_str())
}

/// The icon for a file of this name.
#[must_use]
pub(crate) fn file_icon(name: &str) -> IconName {
    match extension(name).as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" | "bmp" | "tif" | "tiff"
        | "svg" => IconName::Image,
        "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm" => IconName::Film,
        "mp3" | "m4a" | "wav" | "aac" | "flac" | "ogg" => IconName::Music,
        "zip" | "gz" | "tgz" | "tar" | "7z" | "rar" => IconName::FileArchive,
        "csv" | "tsv" | "xls" | "xlsx" | "numbers" => IconName::FileSpreadsheet,
        "pdf" | "txt" | "md" | "rtf" | "doc" | "docx" | "pages" | "html" | "htm" => {
            IconName::FileText
        }
        _ => IconName::File,
    }
}

/// The chips, in a row that scrolls sideways when they don't fit; nothing when there are none.
/// A video's still frames have no chips of their own: the video's shows the first.
pub(crate) fn chips(store: &Entity<Store>, p: &Pal, cx: &App) -> Option<AttachmentGroup> {
    let attachments = store.read(cx).attachments();
    let chips: Vec<Chip> = attachments
        .shown()
        .map(|item| chip(item, attachments.poster(item.id), store))
        .collect();
    if chips.is_empty() {
        return None;
    }
    Some(
        AttachmentGroup::new("attachments")
            .with_edge_fade(p.surface)
            .children(chips),
    )
}

/// One attachment's chip; `poster` is a video's first still frame.
fn chip(item: &Attachment, poster: Option<&Arc<Image>>, store: &Entity<Store>) -> Chip {
    let status = match item.stage {
        Stage::Uploading { .. } => AttachmentStatus::Uploading,
        Stage::Ready(_) => AttachmentStatus::Complete,
        Stage::Failed { .. } => AttachmentStatus::Failed,
    };
    let size = crate::format::file_size(item.size);
    let (description, tip) = match &item.stage {
        Stage::Failed { reason, .. } => (
            reason.clone(),
            format!("Couldn't upload {}: {reason}", item.name),
        ),
        Stage::Uploading { .. } | Stage::Ready(_) => {
            (size.clone(), format!("{} · {size}", item.name))
        }
    };
    let id = item.id;
    let (remove, retry) = (store.clone(), store.clone());
    let mut chip = Chip::new()
        .id(ElementId::Name(format!("attachment-{id}").into()))
        .status(status)
        .tooltip(tip)
        .on_remove(move |_, _, cx| remove.update(cx, |store, cx| store.remove_attachment(id, cx)));
    // No retry for a file that can never go (over the server's limit): its reason stays.
    if matches!(
        item.stage,
        Stage::Failed {
            retryable: true,
            ..
        }
    ) {
        chip = chip.on_retry(move |_, _, cx| {
            retry.update(cx, |store, cx| store.retry_attachment(id, cx));
        });
    }
    if let Some(progress) = item.progress() {
        chip = chip.progress(progress * 100.0);
    }
    match thumbnail(item, poster) {
        // A video's first still frame, with a play mark over it.
        Some(source) if crate::store::stills::is_video(&item.name) => chip.media(
            AttachmentMedia::new().src(source).overlay(
                Icon::new(IconName::CirclePlay)
                    .size(px(18.))
                    .text_color(white()),
            ),
        ),
        Some(source) => chip.media(AttachmentMedia::new().src(source)),
        None => chip
            .media(AttachmentMedia::new().child(Icon::new(file_icon(&item.name))))
            .content(
                AttachmentContent::new()
                    .title(AttachmentTitle::new(item.name.clone()).status(status))
                    .description(AttachmentDescription::new(description).status(status)),
            ),
    }
}

/// What a picture's chip shows: the pasted picture, the file it reads, or a video's first
/// still frame.
fn thumbnail(item: &Attachment, poster: Option<&Arc<Image>>) -> Option<ImageSource> {
    if let Some(picture) = item.picture.as_ref().or(poster) {
        return Some(ImageSource::from(Arc::clone(picture)));
    }
    match &item.source {
        UploadSource::File(path) if is_drawn(&item.name) => Some(ImageSource::from(path.clone())),
        UploadSource::File(_) | UploadSource::Bytes(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pictures_this_app_draws_get_a_thumbnail() {
        assert!(is_drawn("Screenshot.PNG"));
        assert!(is_drawn("photo.jpeg"));
        assert!(!is_drawn("IMG_0001.HEIC"), "no HEIC decoder");
        assert!(!is_drawn("notes"));
    }

    #[test]
    fn a_file_gets_the_icon_of_its_kind() {
        assert!(matches!(file_icon("IMG_0001.HEIC"), IconName::Image));
        assert!(matches!(file_icon("clip.mov"), IconName::Film));
        assert!(matches!(file_icon("week.pdf"), IconName::FileText));
        assert!(matches!(file_icon("data.csv"), IconName::FileSpreadsheet));
        assert!(matches!(file_icon("Makefile"), IconName::File));
    }
}
