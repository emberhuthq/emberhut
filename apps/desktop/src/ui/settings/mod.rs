//! Settings: a front door that asks first (a search over every setting, the groups as chips,
//! suggestions worked out from how things stand) and a page per group (Your ember,
//! Appearance, Devices, Connections, Sharing, Privacy, Advanced, the account) built of the
//! rows and sections each module here draws, with the pages those rows open. Every page is
//! one of [`SettingsPane`], over the front door in the window's column, so Back returns to it.

mod about;
mod account;
mod appearance;
mod chrome;
mod chrome_profile;
mod chrome_sheets;
pub(crate) mod environment;
mod front_door;
mod group;
mod pages;
pub(crate) mod parts;
mod places;
mod share;
mod switches;
mod updates;
mod usage;

use std::collections::HashSet;
use std::time::Instant;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, FocusHandle, ScrollHandle, Window, div};

use self::about::AboutPane;
use self::appearance::AppearancePane;
pub(crate) use self::appearance::OpenAgentRoom;
pub(crate) use self::chrome::ChromePane;
use self::front_door::{Listing, PLACEHOLDER};
use self::group::FLASH;
use self::parts::{nav_row, page_body};
use super::column;
use super::ember::credentials::CredentialsView;
use super::pairing::PairingForm;
use super::palette::{Pal, category_color, pal};
use super::qr::{PairingQr, Qr};
use super::widgets::page_head;
use crate::store::credentials::{not_in_use, saved_line};
use crate::store::settings::SettingsPane;
use crate::store::settings::chrome::Health;
use crate::store::settings::entries::{Destination, Entry, Fold, Group};
use crate::store::settings::now::Now;
use crate::store::{Metrics, Page, Store};

/// Settings: its front door, and the group pages and the pages their rows open.
pub(crate) struct SettingsView {
    store: Entity<Store>,
    pairing: Entity<PairingForm>,
    appearance: Entity<AppearancePane>,
    about: Entity<AboutPane>,
    chrome: Entity<ChromePane>,
    credentials: Entity<CredentialsView>,
    /// The account's sign-in sheet's focus.
    account_sheet: FocusHandle,
    /// The pairing link last drawn as a QR code.
    qr: PairingQr,
    /// The front door's search field.
    search: Entity<InputState>,
    /// What the field holds.
    query: String,
    /// The highlighted row of the front door's list.
    pick: usize,
    /// The front door's own focus: the second Esc leaves the field for it.
    focus: FocusHandle,
    /// The disclosures open on the group pages.
    folds: HashSet<Fold>,
    /// The setting just opened from the front door, and when: its block is washed briefly.
    flash: Option<(&'static str, Instant)>,
    /// The setting whose block the next group page drawn scrolls into view.
    scroll_to: Option<&'static str>,
    /// The group pages' scroll.
    group_scroll: ScrollHandle,
    /// The group page last drawn, so another starts at its top.
    shown_group: Option<Group>,
    /// Whether Clear… on the dark room's history was pressed and waits on its confirmation.
    dark_clear_asked: bool,
}

impl SettingsView {
    /// Settings over `store`, with the pairing form it shares with the sign-in screen.
    pub(crate) fn new(
        store: Entity<Store>,
        pairing: Entity<PairingForm>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        if let Some(signed) = crate::account::installed(cx) {
            cx.observe(&signed, |_, _, cx| cx.notify()).detach();
        }
        let appearance = cx.new(|cx| AppearancePane::new(store.clone(), window, cx));
        let about = cx.new(|cx| AboutPane::new(store.clone(), window, cx));
        let chrome = cx.new(|cx| ChromePane::new(store.clone(), window, cx));
        // Once a provider is saved, back to where the picker was opened from.
        let credentials = cx.new(|cx| {
            CredentialsView::new(
                store.clone(),
                |window, cx| {
                    if let Some(column) = column::of(window, cx) {
                        column.update(cx, column::Column::back);
                    }
                },
                window,
                cx,
            )
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(PLACEHOLDER));
        cx.subscribe_in(
            &search,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.query = input.read(cx).value().to_string();
                    this.pick = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.open_picked(window, cx),
                _ => {}
            },
        )
        .detach();
        Self {
            store,
            pairing,
            appearance,
            about,
            chrome,
            credentials,
            account_sheet: cx.focus_handle(),
            qr: PairingQr::default(),
            search,
            query: String::new(),
            pick: 0,
            focus: cx.focus_handle(),
            folds: HashSet::new(),
            flash: None,
            scroll_to: None,
            group_scroll: ScrollHandle::new(),
            shown_group: None,
            dark_clear_asked: false,
        }
    }

    /// The front door shows in the window's column: the search field takes the keyboard. Opened
    /// `afresh` (not on the way back from one of its pages), it starts empty.
    pub(crate) fn opened(&mut self, afresh: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.search.update(cx, |field, cx| {
            if afresh {
                field.set_value("", window, cx);
            }
            field.focus(window, cx);
        });
        if afresh {
            self.query.clear();
            self.pick = 0;
        }
        cx.notify();
    }

    /// How things stand now, for the front door and the group pages.
    fn now(&self, cx: &App) -> Now {
        let has_account = crate::account::installed(cx).is_some();
        self.store.read(cx).settings_now(pal(cx).dark, has_account)
    }

    /// A row that opens one of Settings' pages, over what the column shows (Back returns).
    fn opens(
        pane: SettingsPane,
    ) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
        move |_, window, cx| column::push(Page::SettingsPane(pane), window, cx)
    }

    /// Opens `group`'s page, at its top.
    fn open_group(&mut self, group: Group, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.scroll_to = None;
        self.shown_group = None;
        column::push(Page::SettingsPane(group.pane()), window, cx);
    }

    /// Opens where `entry` lives: its group's page scrolled to its row, washed for a moment
    /// (its disclosure opened), or the page it has of its own. Nothing changes by itself.
    fn go(&mut self, entry: &'static Entry, window: &mut Window, cx: &mut Context<'_, Self>) {
        let page = match entry.to {
            Destination::Group => {
                if let Some(fold) = entry.fold {
                    self.folds.insert(fold);
                }
                self.shown_group = None;
                self.scroll_to = Some(entry.id);
                self.flash = Some((entry.id, Instant::now()));
                // Drawn again once the wash is over.
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(FLASH).await;
                    if let Err(error) = this.update(cx, |_, cx| cx.notify()) {
                        log::debug!("Settings closed before its wash ended: {error}");
                    }
                })
                .detach();
                Page::SettingsPane(entry.group.pane())
            }
            Destination::Pane(pane) => Page::SettingsPane(pane),
            Destination::Lessons => {
                if !self.store.read(cx).facts().learning_open() {
                    self.store.update(cx, Store::toggle_learning);
                }
                Page::SettingsPane(SettingsPane::AboutYou)
            }
            Destination::Machines => Page::Machines,
        };
        column::push(page, window, cx);
        cx.notify();
    }

    /// Opens the highlighted row of the front door's list.
    fn open_picked(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let listing = Listing::of(&self.query, &self.now(cx));
        let rows = listing.rows();
        if let Some(row) = rows.get(self.pick.min(rows.len().saturating_sub(1))) {
            self.go(row.entry, window, cx);
        }
    }

    /// Moves the highlight `step` rows, staying in the list.
    fn step(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        let count = Listing::of(&self.query, &self.now(cx)).rows().len();
        let last = count.saturating_sub(1);
        self.pick = self.pick.min(last).saturating_add_signed(step).min(last);
        cx.notify();
    }

    /// Esc in the field: clears it; empty, lets go of it.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.query.is_empty() {
            window.focus(&self.focus, cx);
        } else {
            self.search
                .update(cx, |field, cx| field.set_value("", window, cx));
            self.query.clear();
            self.pick = 0;
        }
        cx.notify();
    }

    /// One of Settings' pages, as the column shows it over the front door (its bar goes Back):
    /// a group's page, or one their rows open. `scale` is the display's, for the QR code.
    pub(crate) fn pane(
        &mut self,
        pane: SettingsPane,
        scale: f32,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let p = pal(cx);
        let now = self.now(cx);
        let blocks = match pane {
            SettingsPane::Appearance => return Self::titled(pane, self.appearance.clone(), &p),
            SettingsPane::ChromeProfiles => return Self::titled(pane, self.chrome.clone(), &p),
            SettingsPane::AboutYou => return Self::titled(pane, self.about.clone(), &p),
            SettingsPane::Credentials => return Self::titled(pane, self.credentials.clone(), &p),
            SettingsPane::Ember => self.ember_blocks(&now, &p, cx),
            SettingsPane::Devices => self.devices_blocks(scale, &p, cx),
            SettingsPane::Connections => self.connections_blocks(&now, &p, cx),
            SettingsPane::Sharing => Self::sharing_blocks(&p),
            SettingsPane::Privacy => self.privacy_blocks(&now, &p, cx),
            SettingsPane::Advanced => self.advanced_blocks(&p, cx),
            SettingsPane::Account => Self::account_blocks(&p, cx),
        };
        let Some(group) = Group::of_pane(pane) else {
            return div().into_any_element();
        };
        let drawn = self.group_page(group, group.status(&now), blocks, &p);
        let sheet = (pane == SettingsPane::Account)
            .then(|| self.account_sheet(&p, cx))
            .flatten();
        div()
            .size_full()
            .relative()
            .child(drawn)
            .children(sheet)
            .into_any_element()
    }

    /// A page of its own (Appearance, Chrome profiles, what your ember knows, what it thinks
    /// with) under its title.
    fn titled(pane: SettingsPane, body: impl IntoElement, p: &Pal) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(page_head(p, pane.title(), None))
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// The account's sign-in sheet, while it shows a code.
    fn account_sheet(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        crate::account::installed(cx)
            .and_then(|signed| account::sheet_for(&signed, &self.account_sheet, p, cx))
            .map(IntoElement::into_any_element)
    }

    /// The QR code of the pairing link made for another device, for a display of `scale`.
    fn pairing_qr(&mut self, scale: f32, cx: &mut Context<'_, Self>) -> Option<Qr> {
        let code = self.store.read(cx).new_code().cloned();
        self.qr.of(code.as_ref(), scale, cx)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let now = self.now(cx);
        let sheet = self.account_sheet(&p, cx);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .child(page_head(&p, "Settings", None))
            .child(page_body("settings-scroll", self.front_door(&now, &p, cx)))
            .children(sheet)
    }
}

/// The row of what the ember thinks with: the provider saved, its model and key, in the colour
/// of whether the ember runs on it now; it opens the picker.
fn thinks_with_row(store: &Store, p: &Pal) -> AnyElement {
    let picker = store.credentials();
    let (status, health, detail) = match picker.saved() {
        None if picker.read_problem().is_some() => {
            ("Couldn't read it".to_owned(), Health::Warn, String::new())
        }
        None => ("Reading…".to_owned(), Health::Off, String::new()),
        Some(saved) => {
            let health = match (&saved.provider, not_in_use(saved)) {
                (None, _) => Health::Off,
                (Some(_), Some(_)) => Health::Warn,
                (Some(_), None) => Health::Fine,
            };
            let detail = not_in_use(saved).unwrap_or_default().to_owned();
            (saved_line(saved), health, detail)
        }
    };
    nav_row(
        "credentials-row",
        p,
        (IconName::Brain, category_color(2)),
        "What your ember thinks with",
        (status, health),
        &detail,
    )
    .on_click(SettingsView::opens(SettingsPane::Credentials))
    .test_support()
    .into_any_element()
}

/// The poll's numbers, for the Server group.
fn metrics_line(metrics: &Metrics) -> String {
    format!(
        "First state after {} ms · the last new one fetched and decoded in {} ms · {} polls, {} unchanged (304)",
        metrics.first_state().unwrap_or_default().as_millis(),
        metrics.last_fetch().unwrap_or_default().as_millis(),
        metrics.polls(),
        metrics.unchanged_polls()
    )
}

#[cfg(test)]
mod tests;
