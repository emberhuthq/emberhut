//! A thumbnail on a task's page: a picture cropped to fill a box of a fixed size, and never
//! drawn past it.
//!
//! GPUI's `img` takes its picture's aspect ratio unless it is given one. Inside a block box (a
//! `div` that isn't `flex`), a fixed width then decided its height whatever height it was given,
//! so a phone video's 9:16 frame in a 120 × 84 box came out 120 × 213, and with nothing clipping
//! it, it was painted over what followed. Here the picture takes the box's own ratio, the box is
//! a flex box that clips what it holds, and the picture is cropped to cover it.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{Div, Hsla, ImageSource, ObjectFit, RenderImage, div, img, px};

/// `picture` cropped to fill a `width` × `height` box with corners of `radius`; with no picture
/// yet, the box filled with `fill`. `selector` names the picture for tests.
pub(super) fn thumb(
    picture: Option<Arc<RenderImage>>,
    (width, height): (f32, f32),
    radius: f32,
    fill: Hsla,
    selector: impl FnOnce() -> String,
) -> Div {
    div()
        .flex()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .rounded(px(radius))
        .overflow_hidden()
        .bg(fill)
        .children(picture.map(|image| {
            img(ImageSource::Render(image))
                .debug_selector(selector)
                .flex_none()
                .w(px(width))
                .h(px(height))
                .aspect_ratio(width / height)
                .object_fit(ObjectFit::Cover)
        }))
}
