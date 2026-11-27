//! A PDF in the viewer: its pages one after another, fitted to the width and zoomed from there,
//! with the page the reader is on (`3 / 12`) in the corner. Only the pages in sight are drawn
//! (by the platform, [`crate::pdf`], one at a time off the UI thread), each as sharp as it
//! shows, and pages well out of sight are let go again.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Bounds, FontWeight, Image, ImageSource, Pixels, RenderImage, ScrollHandle,
    Window, div, img, px,
};

use super::body::Body;
use super::pages::{GAP, Layout, PageSize, label, sharpness};
use super::zoom::Zoom;
use super::{FileViewer, free_image};
use crate::ui::palette::pal;

/// How many pages beyond those in sight stay drawn each way.
const KEPT_AROUND: usize = 4;

/// The narrowest a page shows when fitted.
const NARROWEST: f32 = 120.;

/// A PDF's pages and the ones drawn.
pub(super) struct PdfPages {
    path: PathBuf,
    sizes: Vec<PageSize>,
    /// The pages drawn: how sharp, and the picture.
    drawn: HashMap<usize, (u32, Arc<RenderImage>)>,
    /// The page being drawn now.
    drawing: Option<usize>,
    /// Pages that couldn't be drawn (not tried again while the PDF shows).
    failed: HashSet<usize>,
}

impl PdfPages {
    /// The PDF kept at `path`, whose pages are `sizes` big.
    #[must_use]
    pub(super) fn new(path: PathBuf, sizes: Vec<PageSize>) -> Self {
        Self {
            path,
            sizes,
            drawn: HashMap::new(),
            drawing: None,
            failed: HashSet::new(),
        }
    }

    /// Frees every page drawn.
    pub(super) fn free(self, cx: &mut App) {
        for (_, (_, image)) in self.drawn {
            free_image(image, cx);
        }
    }

    /// Lets go of the pages drawn outside `keep`.
    fn let_go(&mut self, keep: &Range<usize>, cx: &mut App) {
        let gone: Vec<usize> = self
            .drawn
            .keys()
            .copied()
            .filter(|page| !keep.contains(page))
            .collect();
        for page in gone {
            if let Some((_, image)) = self.drawn.remove(&page) {
                free_image(image, cx);
            }
        }
    }

    /// Draws the first page of `pages` that isn't drawn at `sharp` yet, unless one is being
    /// drawn.
    fn draw_next(&mut self, pages: Range<usize>, sharp: u32, cx: &mut Context<'_, FileViewer>) {
        if self.drawing.is_some() {
            return;
        }
        let Some(page) = pages.into_iter().find(|page| {
            !self.failed.contains(page) && self.drawn.get(page).map(|(at, _)| *at) != Some(sharp)
        }) else {
            return;
        };
        self.drawing = Some(page);
        let path = self.path.clone();
        let svg = cx.svg_renderer();
        cx.spawn(async move |this, cx| {
            let draw_path = path.clone();
            let drawn = cx
                .background_spawn(async move {
                    let drawn =
                        crate::pdf::draw(&draw_path, page, sharp).map_err(|e| e.to_string())?;
                    Image::from_bytes(drawn.format, drawn.bytes)
                        .to_image_data(svg)
                        .map_err(|error| error.to_string())
                })
                .await;
            let updated = this.update(cx, |viewer, cx| {
                let pages = viewer.open.as_mut().and_then(|open| match &mut open.body {
                    Body::Pdf(pages) if pages.path == path => Some(pages),
                    _ => None,
                });
                match (pages, drawn) {
                    (Some(pages), drawn) => pages.finished(page, sharp, drawn, cx),
                    (None, Ok(image)) => free_image(image, cx),
                    (None, Err(_)) => {}
                }
                cx.notify();
            });
            if let Err(error) = updated {
                log::debug!("the viewer closed while a page was drawn: {error}");
            }
        })
        .detach();
    }

    /// Page `page` came back drawn at `sharp`, or why it couldn't be.
    fn finished(
        &mut self,
        page: usize,
        sharp: u32,
        drawn: Result<Arc<RenderImage>, String>,
        cx: &mut App,
    ) {
        self.drawing = None;
        match drawn {
            Ok(image) => {
                if let Some((_, old)) = self.drawn.insert(page, (sharp, image)) {
                    free_image(old, cx);
                }
            }
            Err(why) => {
                log::warn!(
                    "couldn't draw page {} of {}: {why}",
                    page + 1,
                    self.path.display()
                );
                self.failed.insert(page);
            }
        }
    }

    /// The pages, as the body, `area` big, at `zoom`.
    pub(super) fn view(
        &mut self,
        area: Bounds<Pixels>,
        zoom: Zoom,
        scroll: &ScrollHandle,
        window: &Window,
        cx: &mut Context<'_, FileViewer>,
    ) -> AnyElement {
        let p = pal(cx);
        let room = f32::from(area.size.width);
        let fitted = (room - 2. * GAP).max(NARROWEST);
        let layout = Layout::new(&self.sizes, fitted * zoom.scale());
        let offset = -f32::from(scroll.offset().y);
        let viewport = f32::from(area.size.height);
        let in_sight = layout.in_sight(offset, viewport);
        let keep = in_sight.start.saturating_sub(KEPT_AROUND)..in_sight.end + KEPT_AROUND;
        self.let_go(&keep, cx);
        let sharp = sharpness(layout.width(), window.scale_factor());
        self.draw_next(in_sight.clone(), sharp, cx);
        let width = px(layout.width());
        let pages = layout.heights().enumerate().map(|(page, height)| {
            let shown = in_sight.contains(&page);
            div()
                .w(width)
                .h(px(height))
                .flex_none()
                .bg(gpui_kit::white())
                .shadow_md()
                .when_some(self.drawn.get(&page).filter(|_| shown), |d, (_, image)| {
                    d.child(
                        img(ImageSource::Render(Arc::clone(image)))
                            .w(width)
                            .h(px(height)),
                    )
                })
                .when(self.failed.contains(&page), |d| {
                    d.flex()
                        .items_center()
                        .justify_center()
                        .text_color(p.muted)
                        .child("Couldn't draw this page")
                })
        });
        let current = label(layout.current(offset, viewport), self.sizes.len());
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("viewer-pdf")
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(scroll)
                    .bg(p.card)
                    .child(
                        div()
                            .w(px(room.max(layout.width() + 2. * GAP)))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(GAP))
                            .py(px(GAP))
                            .children(pages),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(12.))
                    .right(px(12.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded_full()
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.border)
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(current),
            )
            .into_any_element()
    }
}
