//! A picture in the viewer: fitted to the body on a black ground, zoomed from there, and
//! scrolled both ways once it is bigger than the body. A double-click goes close and back, a
//! pinch follows the fingers.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Bounds, ClickEvent, ImageSource, PinchEvent, Pixels, RenderImage, ScrollHandle,
    Size, div, img, px, size,
};

use super::FileViewer;
use super::zoom::Zoom;

/// How big `image` shows in a body `area` big at `zoom`: fitted inside it (larger or smaller),
/// then zoomed.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a picture's sides are far below the 2^24 pixels an f32 holds exactly"
)]
pub(super) fn shown_size(image: &RenderImage, area: Size<Pixels>, zoom: Zoom) -> Size<Pixels> {
    let natural = image.size(0);
    let (width, height) = (
        natural.width.0.max(1) as f32,
        natural.height.0.max(1) as f32,
    );
    let fit = fit_scale(width, height, f32::from(area.width), f32::from(area.height));
    size(
        px(width * fit * zoom.scale()),
        px(height * fit * zoom.scale()),
    )
}

/// The scale that fits a `width` by `height` picture inside an area `room_width` by
/// `room_height` (1 when the area isn't known yet).
#[must_use]
fn fit_scale(width: f32, height: f32, room_width: f32, room_height: f32) -> f32 {
    if room_width <= 0. || room_height <= 0. || width <= 0. || height <= 0. {
        return 1.;
    }
    (room_width / width).min(room_height / height)
}

/// The picture, as the body.
pub(super) fn view(
    area: Bounds<Pixels>,
    zoom: Zoom,
    scroll: &ScrollHandle,
    image: &Arc<RenderImage>,
    cx: &mut Context<'_, FileViewer>,
) -> AnyElement {
    let shown = shown_size(image, area.size, zoom);
    div()
        .id("viewer-picture")
        .size_full()
        .overflow_scroll()
        .track_scroll(scroll)
        .bg(gpui_kit::black())
        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
            if event.click_count() == 2
                && let Some(open) = &mut this.open
            {
                open.zoom.toggle();
                open.centre();
                cx.notify();
            }
        }))
        .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
            if let Some(open) = &mut this.open {
                open.zoom.pinch(event.delta);
                open.centre();
                cx.notify();
            }
        }))
        .child(
            // Sized outright, not by `min_w`: a block parent stretches a child to its own width,
            // and the scroll range is the direct child's bounds, so a picture wider than the
            // body would overflow both sides with no room to scroll sideways.
            div()
                .w(shown.width.max(area.size.width))
                .h(shown.height.max(area.size.height))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(ImageSource::Render(Arc::clone(image)))
                        .w(shown.width)
                        .h(shown.height)
                        .flex_none(),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_fits_inside_its_area_either_way() {
        assert!(
            (fit_scale(2000., 1000., 1000., 1000.) - 0.5).abs() < 1e-6,
            "a wide one by width"
        );
        assert!(
            (fit_scale(500., 1000., 1000., 500.) - 0.5).abs() < 1e-6,
            "a tall one by height"
        );
        assert!(
            (fit_scale(100., 100., 400., 300.) - 3.).abs() < 1e-6,
            "a small one grows"
        );
        assert!(
            (fit_scale(100., 100., 0., 0.) - 1.).abs() < 1e-6,
            "no area yet"
        );
    }
}
