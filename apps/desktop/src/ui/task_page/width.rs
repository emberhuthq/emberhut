//! How wide the column beside the chat is, whether it shows a page or a task's: one rule for
//! both. The chat beside it, and a report open beside both, have their narrowest widths first;
//! the column takes the rest, up to its own share of the window (or the width the user dragged
//! its edge to, `ui::edge`). When the rest is narrower than
//! the column's minimum there is no width for it beside the chat: it takes the chat's place
//! (`ui::column::beside`) until Back or Close brings the chat back, or the window or the
//! sidebar leaves room for both.

use gpui_kit::{Pixels, px};

use crate::ui::report::{MIN_CHAT, MIN_PANEL};

/// The share of the window the column takes while there is room.
const SHARE: f32 = 0.42;

/// The narrowest and the widest the column's share of the window is taken as.
const WANTED: (f32, f32) = (400., 620.);

/// The narrowest the column shows at.
const MIN_COLUMN: Pixels = px(280.);

/// The column's width in whole pixels, in a window `window` wide with `content` of it beside
/// the sidebar, a report open beside the page if `report`: the width the user `dragged` it to
/// (never under [`MIN_COLUMN`]), else its share of the window, as far as the room left allows;
/// `None` when that leaves it less than [`MIN_COLUMN`].
#[must_use]
pub(crate) fn column_width(
    window: Pixels,
    content: Pixels,
    report: bool,
    dragged: Option<Pixels>,
) -> Option<Pixels> {
    let share = || px((f32::from(window) * SHARE).clamp(WANTED.0, WANTED.1));
    let wanted = dragged.map_or_else(share, |dragged| dragged.max(MIN_COLUMN));
    let others = if report {
        MIN_CHAT + MIN_PANEL
    } else {
        MIN_CHAT
    };
    let room = content - others;
    (room >= MIN_COLUMN).then(|| wanted.min(room).round())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The column in a window `window` wide with the sidebar's 260 px on the left.
    fn beside_sidebar(window: f32, report: bool) -> Option<Pixels> {
        column_width(px(window), px(window - 260.), report, None)
    }

    #[test]
    fn with_room_the_column_takes_its_share() {
        assert_eq!(beside_sidebar(1440., false), Some(px(605.)), "42% of 1440");
        assert_eq!(beside_sidebar(2400., false), Some(px(620.)), "at most 620");
        assert_eq!(
            column_width(px(1000.), px(1000.), false, None),
            Some(px(420.)),
            "no sidebar"
        );
    }

    #[test]
    fn the_page_keeps_its_minimum() {
        // 640 px beside the sidebar: 360 for the page, 280 for the column.
        assert_eq!(beside_sidebar(900., false), Some(px(280.)));
        assert_eq!(beside_sidebar(1100., false), Some(px(462.)), "42% is 462");
        assert_eq!(beside_sidebar(1000., false), Some(px(380.)), "42% is 420");
    }

    #[test]
    fn a_report_beside_the_page_has_its_minimum_first() {
        // 1180 px beside the sidebar: 360 for the page, 380 for the report, 440 left.
        assert_eq!(beside_sidebar(1440., true), Some(px(440.)));
        assert_eq!(beside_sidebar(1800., true), Some(px(620.)));
        assert_eq!(beside_sidebar(900., true), None, "no room left");
    }

    #[test]
    fn below_its_minimum_the_column_doesnt_show() {
        assert_eq!(beside_sidebar(899., false), None);
        assert_eq!(beside_sidebar(600., false), None);
        assert_eq!(beside_sidebar(1279., true), None);
        assert_eq!(beside_sidebar(1280., true), Some(MIN_COLUMN));
    }

    #[test]
    fn a_dragged_width_holds_while_it_fits() {
        let dragged = |width: f32, window: f32| {
            column_width(px(window), px(window - 260.), false, Some(px(width)))
        };
        assert_eq!(dragged(700., 1440.), Some(px(700.)), "wider than its share");
        assert_eq!(dragged(300., 1440.), Some(px(300.)));
        assert_eq!(
            dragged(100., 1440.),
            Some(MIN_COLUMN),
            "never under its minimum"
        );
        // 1180 px beside the sidebar, the chat keeps 360.
        assert_eq!(
            dragged(1000., 1440.),
            Some(px(820.)),
            "the chat keeps its room"
        );
        assert_eq!(dragged(500., 899.), None, "no room beside the chat");
    }
}
