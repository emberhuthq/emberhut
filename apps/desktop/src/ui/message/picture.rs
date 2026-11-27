//! A picture in a message: drawn at its own shape within 320 × 360 points, at an explicit size,
//! so the space the layout gives it is the space it is painted in.
//!
//! GPUI's `img` left to size itself clamps its width and height to their maximums separately,
//! while the layout engine then derives the height from the clamped width and the picture's
//! aspect ratio, past the maximum height. The message made room for the clamped box and the
//! picture was painted taller, over what came after it. Sized here instead, both agree.
//!
//! A picture's pixels count as points, as in browsers and in GPUI's own `img`: a 2× screenshot
//! shows at its pixel size, which the 320 × 360 box then caps.

use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    DevicePixels, ElementId, Entity, ImageSource, ObjectFit, Pixels, RenderImage, Size, div, img,
    px, size,
};

use super::{remote, view_in};
use crate::store::Store;
use crate::ui::chat::ChatView;
use crate::ui::files::actions as file_actions;
use crate::ui::files::drag;
use crate::ui::palette::Pal;
use emberhut_api::chat::{ChatFile, ChatMessage};

/// The most room a picture takes in a message.
const MAX: Size<Pixels> = size(px(320.), px(360.));

/// What a picture takes while it loads (or when it can't be shown).
const PLACEHOLDER: Size<Pixels> = size(px(240.), px(140.));

/// How round a picture's corners are.
const CORNER: Pixels = px(10.);

/// The size a picture of `natural` size (in points) is shown at within `max`: scaled down to fit
/// it whole, keeping its shape, and never scaled up. `None` when the picture has no area.
#[must_use]
fn fit(natural: Size<Pixels>, max: Size<Pixels>) -> Option<Size<Pixels>> {
    let (width, height) = (natural.width.as_f32(), natural.height.as_f32());
    let shows = |side: f32| side.is_finite() && side > 0.;
    if !shows(width) || !shows(height) {
        return None;
    }
    let scale = (max.width.as_f32() / width)
        .min(max.height.as_f32() / height)
        .clamp(0., 1.);
    Some(size(px(width * scale), px(height * scale)))
}

/// A decoded picture's size in points: its first frame's pixels, a point each.
#[must_use]
fn natural(image: &RenderImage) -> Size<Pixels> {
    let pixels = image.size(0);
    let points = |side: DevicePixels| Pixels::from(u32::from(side));
    size(points(pixels.width), points(pixels.height))
}

/// Message `message`'s picture `i` (`file`), at its own shape once it has come, a placeholder
/// until then; the viewer opens it on a click, its uses are on a right-click, and it drags out
/// as the file.
pub(super) fn picture(
    message: &ChatMessage,
    i: usize,
    file: &ChatFile,
    store: &Entity<Store>,
    p: &Pal,
    cx: &Context<'_, ChatView>,
) -> impl IntoElement {
    let name = format!("picture-{}-{i}", message.id);
    let image = store
        .read(cx)
        .picture(&file.url)
        .and_then(|image| fit(natural(&image), MAX).map(|shown| (image, shown)));
    let shown = match image {
        Some((image, shown)) => img(ImageSource::Render(image))
            .debug_selector(|| name.clone())
            .w(shown.width)
            .h(shown.height)
            .object_fit(ObjectFit::Contain)
            .rounded(CORNER)
            .into_any_element(),
        None => div()
            .debug_selector(|| name.clone())
            .w(PLACEHOLDER.width)
            .h(PLACEHOLDER.height)
            .rounded(CORNER)
            .bg(p.card)
            .into_any_element(),
    };
    let file = remote(file);
    let element = div()
        .id(ElementId::Name(name.into()))
        // Never squeezed by the message's column: the column grows to hold it.
        .flex_none()
        .cursor_pointer()
        .child(shown);
    let opened = file.clone();
    drag::draggable(element, &file, store)
        .on_click(move |_, window, cx| view_in(window, opened.clone(), cx))
        .context_menu(file_actions::menu(&file, store))
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Bounds, point};
    use image::{Frame, RgbaImage};

    use super::*;

    fn points(width: f32, height: f32) -> Size<Pixels> {
        size(px(width), px(height))
    }

    fn close(a: Size<Pixels>, b: Size<Pixels>) -> bool {
        (a.width - b.width).abs() < px(0.01) && (a.height - b.height).abs() < px(0.01)
    }

    #[test]
    fn a_tall_screenshot_fits_the_height_at_its_shape() {
        // A phone's 2× screenshot, 1170 × 2532 pixels.
        let shown = fit(points(1170., 2532.), MAX).expect("a size");
        assert!(
            close(shown, points(1170. * 360. / 2532., 360.)),
            "{shown:?}"
        );
    }

    #[test]
    fn a_wide_picture_fits_the_width_at_its_shape() {
        let shown = fit(points(1600., 400.), MAX).expect("a size");
        assert!(close(shown, points(320., 80.)), "{shown:?}");
    }

    #[test]
    fn a_small_picture_is_not_scaled_up() {
        let shown = fit(points(48., 30.), MAX).expect("a size");
        assert!(close(shown, points(48., 30.)), "{shown:?}");
        let exact = fit(MAX, MAX).expect("a size");
        assert!(close(exact, MAX), "{exact:?}");
    }

    #[test]
    fn a_picture_without_area_has_no_size() {
        for natural in [
            points(0., 0.),
            points(0., 100.),
            points(100., 0.),
            points(-5., 100.),
            points(f32::NAN, 100.),
            points(f32::INFINITY, 100.),
        ] {
            assert_eq!(fit(natural, MAX), None, "{natural:?}");
        }
        let nowhere = fit(points(100., 50.), points(0., 0.)).expect("a size");
        assert!(close(nowhere, points(0., 0.)), "no room: {nowhere:?}");
    }

    fn picture(width: u32, height: u32) -> RenderImage {
        RenderImage::new(vec![Frame::new(RgbaImage::new(width, height))])
    }

    #[test]
    fn a_pictures_pixels_count_as_points() {
        assert!(close(natural(&picture(1170, 2532)), points(1170., 2532.)));
        assert!(close(natural(&picture(48, 30)), points(48., 30.)));
        assert_eq!(
            fit(natural(&RenderImage::new(vec![])), MAX),
            None,
            "no frame"
        );
    }

    #[test]
    fn the_picture_is_painted_in_the_box_it_is_given() {
        // GPUI paints with the object fit inside the element's bounds: at the fitted size, that
        // is the whole box, so nothing is cut off or drawn outside it.
        for (width, height) in [(1170, 2532), (1600, 400), (40, 40)] {
            let image = picture(width, height);
            let shown = fit(natural(&image), MAX).expect("a size");
            let bounds = Bounds::new(point(px(10.), px(20.)), shown);
            let painted = ObjectFit::Contain.get_bounds(bounds, image.size(0));
            assert!(close(painted.size, bounds.size), "{width} × {height}");
            assert!(
                (painted.origin.x - bounds.origin.x).abs() < px(0.01)
                    && (painted.origin.y - bounds.origin.y).abs() < px(0.01),
                "{width} × {height}"
            );
        }
    }
}
