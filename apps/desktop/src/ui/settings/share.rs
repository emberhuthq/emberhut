//! Share for a link: the system's share menu where the platform has one ([`crate::ui::share`]);
//! elsewhere Share isn't offered, and Copy covers it.

use gpui_kit::ElementId;
use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;

use crate::ui::share::{self, Shared};

/// Share for `link`, opening the system's share menu at the pointer; `None` where the platform
/// has no share menu.
pub(super) fn button(id: impl Into<ElementId>, link: &str) -> Option<Button> {
    share::HAS_MENU.then(|| {
        let link = link.to_owned();
        Button::new(id)
            .small()
            .icon(IconName::Share)
            .label("Share")
            .on_click(move |_, window, cx| share::share(Shared::Link(link.clone()), window, cx))
    })
}

// On macOS the menu is AppKit's own and needs the Mac to show (see the README).
#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn off_macos_share_is_not_offered() {
        assert!(button("share", "https://example.com/p#AB-12").is_none());
    }
}
