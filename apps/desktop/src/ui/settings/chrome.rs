//! Chrome profiles: each of the user's Chrome profiles with the assistant's extension, paired
//! on its own so the assistant works in the account a task is about. The list, the links still
//! waiting to pair, and adding one; a profile, a new profile's name and a link to pair each
//! open in a sheet over the list (`chrome_sheets`).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Div, ElementId, Entity, FocusHandle, FontWeight, Window, div,
    px,
};
use jiff::tz::TimeZone;

use super::chrome_sheets;
use super::parts::{note, page_body, problem_line, section, tag};
use crate::brand::APP_NAME;
use crate::store::Store;
use crate::store::settings::chrome::{ChromeSheet, badge_index, initial, machine_name, waiting};
use crate::store::settings::words::{clock, seen_ago};
use crate::ui::palette::{Pal, category_color, pal};
use emberhut_api::chrome::{ChromeControl, ChromePairing, ChromeProfile};
use emberhut_api::ids::ChromeProfileId;

/// The Chrome profiles page.
pub(crate) struct ChromePane {
    pub(super) store: Entity<Store>,
    /// A new profile's name.
    pub(super) name: Entity<InputState>,
    /// The open profile's name, being changed.
    pub(super) rename: Entity<InputState>,
    /// The profile `rename` was filled for, and the refused renames seen then.
    named: Option<(ChromeProfileId, u64)>,
    /// "Unpair" asked, waiting for its confirmation.
    pub(super) unpairing: bool,
    /// The sheet's focus, and the sheet it was given to.
    pub(super) focus: FocusHandle,
    focused: Option<ChromeSheet>,
}

impl ChromePane {
    /// The page over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Name, like Work"));
        cx.subscribe(&name, |this, input, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => {
                let label = input.read(cx).value().to_string();
                this.store
                    .update(cx, |store, cx| store.add_chrome(&label, cx));
            }
            InputEvent::Change => cx.notify(),
            _ => {}
        })
        .detach();
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        cx.subscribe(&rename, |this, _, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => this.save_name(cx),
            InputEvent::Change => cx.notify(),
            _ => {}
        })
        .detach();
        Self {
            store,
            name,
            rename,
            named: None,
            unpairing: false,
            focus: cx.focus_handle(),
            focused: None,
        }
    }

    /// Renames the open profile to the name typed, if it is a new one.
    pub(super) fn save_name(&mut self, cx: &mut Context<'_, Self>) {
        let Some(ChromeSheet::Profile(id)) = self.store.read(cx).chrome().sheet().cloned() else {
            return;
        };
        let label = self.rename.read(cx).value().trim().to_owned();
        let same = self
            .store
            .read(cx)
            .chrome_profile(id)
            .is_some_and(|profile| profile.label == label);
        if label.is_empty() || same {
            return;
        }
        self.store.update(cx, |store, cx| {
            store.chrome_act(emberhut_api::chrome::ChromeAction::Rename { id, label }, cx);
        });
    }

    /// Keeps the sheet's fields and focus in step with the sheet open: the name field holds the
    /// profile's name (again, after a refused rename), and a new sheet takes the focus.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let store = self.store.read(cx);
        let sheet = store.chrome().sheet().cloned();
        let refused = store.chrome().refused_renames();
        if let Some(ChromeSheet::Profile(id)) = &sheet {
            let label = store
                .chrome_profile(*id)
                .map(|profile| profile.label.clone());
            if self.named != Some((*id, refused))
                && let Some(label) = label
            {
                self.named = Some((*id, refused));
                self.rename
                    .update(cx, |input, cx| input.set_value(label, window, cx));
            }
        }
        if sheet != self.focused {
            self.focused.clone_from(&sheet);
            self.unpairing = false;
            match sheet {
                Some(ChromeSheet::Add) => {
                    self.name.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                        input.focus(window, cx);
                    });
                }
                Some(_) => window.focus(&self.focus, cx),
                None => {}
            }
        }
    }

    fn profile_row(&self, profile: &ChromeProfile, p: &Pal) -> AnyElement {
        let id = profile.id;
        let store = self.store.clone();
        let detail = [
            profile.machine.as_deref().map(machine_name),
            profile
                .version
                .as_ref()
                .map(|version| format!("v{version}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        div()
            .id(ElementId::Name(format!("chrome-{id}").into()))
            .flex()
            .items_center()
            .gap(px(12.))
            .cursor_pointer()
            .child(badge(profile, 38., p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_size(px(13.))
                                    .truncate()
                                    .child(profile.label.clone()),
                            )
                            .children(profile_tags(profile, p)),
                    )
                    .child(account_line(profile, p))
                    .when(!detail.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(p.muted)
                                .truncate()
                                .child(detail),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(4.))
                    .child(online(profile, p))
                    .when(profile.outdated, |d| d.child(tag("Update", p.warn))),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .size(px(13.))
                    .text_color(p.faint),
            )
            .on_click(move |_, _, cx| {
                store.update(cx, |store, cx| {
                    store.show_chrome_sheet(Some(ChromeSheet::Profile(id)), cx);
                });
            })
            .into_any_element()
    }

    fn waiting_row(&self, pairing: &ChromePairing, p: &Pal) -> AnyElement {
        let until = clock(pairing.until.as_str(), &TimeZone::system()).unwrap_or_default();
        let (cancel, code) = (self.store.clone(), pairing.code.clone());
        div()
            .id(ElementId::Name(
                format!("chrome-link-{}", pairing.code).into(),
            ))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(Spinner::new().small().color(p.muted))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.))
                            .child(pairing.label.clone()),
                    )
                    .child(note(
                        p,
                        format!("Open the link in that Chrome profile · until {until}"),
                    )),
            )
            .child(copy_link(
                ElementId::Name(format!("chrome-link-{}-copy", pairing.code).into()),
                &pairing.link,
                false,
            ))
            .context_menu(move |menu, _, _| {
                let (store, code) = (cancel.clone(), code.clone());
                menu.item(
                    PopupMenuItem::new("Cancel this link").on_click(move |_, _, cx| {
                        let code = code.clone();
                        store.update(cx, |store, cx| store.cancel_chrome_link(code, cx));
                    }),
                )
            })
            .into_any_element()
    }
}

impl Render for ChromePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.sync(window, cx);
        let p = pal(cx);
        let store = self.store.read(cx);
        let profiles = store.chrome_profiles().to_vec();
        let links: Vec<ChromePairing> = store.state().map_or_default(|state| {
            waiting(&state.chrome_pairing)
                .into_iter()
                .cloned()
                .collect()
        });
        let problem = store
            .chrome()
            .sheet()
            .is_none()
            .then(|| store.chrome().problem().map(str::to_owned))
            .flatten();
        let mut rows: Vec<AnyElement> = profiles
            .iter()
            .map(|profile| self.profile_row(profile, &p))
            .collect();
        if profiles.is_empty() {
            rows.push(
                note(
                    &p,
                    format!(
                        "No Chrome profile is paired yet. Add one for each account {APP_NAME} \
                         should work in: Work, Personal…"
                    ),
                )
                .text_size(px(13.))
                .into_any_element(),
            );
        }
        let add = self.store.clone();
        let content = div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(section(
                &p,
                "Paired profiles",
                None,
                rows,
                Some(format!(
                    "Each Chrome profile has its own pairing, so {APP_NAME} works in the account a \
                     task is about. It works there only while you let it in, only in its own \
                     purple tab group, never with passwords, and asks you before a final step."
                )),
            ))
            .when(!links.is_empty(), |d| {
                let rows = links
                    .iter()
                    .map(|link| self.waiting_row(link, &p))
                    .collect();
                d.child(section(&p, "Waiting to pair", None, rows, None))
            })
            .child(
                div().child(
                    Button::new("chrome-add")
                        .icon(gpui_kit::component::IconName::Plus)
                        .label("Add a Chrome profile")
                        .on_click(move |_, _, cx| {
                            add.update(cx, |store, cx| {
                                store.show_chrome_sheet(Some(ChromeSheet::Add), cx);
                            });
                        }),
                ),
            )
            .children(problem.map(|problem| problem_line(&p, problem)));
        let sheet = chrome_sheets::sheet(self, &p, cx);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(page_body("chrome-scroll", content))
            .children(sheet)
    }
}

/// The profile's badge: its initial on its colour (steady for its name), with a green dot
/// while it is online.
pub(super) fn badge(profile: &ChromeProfile, size: f32, p: &Pal) -> Div {
    let fill = category_color(badge_index(&profile.name, 8));
    let p_online = size * 0.28;
    div()
        .relative()
        .size(px(size))
        .flex_none()
        .child(
            div()
                .size_full()
                .rounded_full()
                .bg(fill)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(size * 0.45))
                .font_weight(FontWeight::BOLD)
                .text_color(Pal::readable_on(fill))
                .child(initial(&profile.label)),
        )
        .when(profile.connected, |d| {
            d.child(
                div()
                    .absolute()
                    .right_0()
                    .bottom_0()
                    .size(px(p_online))
                    .rounded_full()
                    .bg(p.success)
                    .border_2()
                    .border_color(p.surface),
            )
        })
}

/// The tags beside a profile's name: the assistant may work there, it is paused, it waits on
/// the user.
pub(super) fn profile_tags(profile: &ChromeProfile, p: &Pal) -> Vec<Div> {
    let mut tags = Vec::new();
    if profile.grant.is_some() {
        tags.push(tag(format!("{APP_NAME} in"), p.accent));
    }
    if profile.paused.is_some() {
        tags.push(tag("Paused", p.muted));
    }
    if profile.control == ChromeControl::Person {
        tags.push(tag("Needs you", p.warn));
    }
    tags
}

/// The Google account, or that it isn't known yet.
pub(super) fn account_line(profile: &ChromeProfile, p: &Pal) -> Div {
    let (text, color) = profile.email.clone().map_or_else(
        || ("Account not known yet".to_owned(), p.faint),
        |email| (email, p.muted),
    );
    div()
        .text_size(px(12.))
        .text_color(color)
        .truncate()
        .child(text)
}

/// Online, or when it was last seen.
pub(super) fn online(profile: &ChromeProfile, p: &Pal) -> Div {
    let text = if profile.connected {
        "Online".to_owned()
    } else {
        seen_ago(
            profile
                .seen
                .as_ref()
                .map(emberhut_api::time::Timestamp::as_str),
            jiff::Timestamp::now(),
        )
    };
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .child(crate::ui::widgets::dot(
            if profile.connected {
                p.success
            } else {
                p.faint
            },
            7.,
        ))
        .child(div().text_size(px(12.)).text_color(p.muted).child(text))
}

/// Copies a link to the clipboard.
pub(super) fn copy_link(id: ElementId, link: &str, prominent: bool) -> Button {
    let link = link.to_owned();
    let button = Button::new(id)
        .small()
        .icon(gpui_kit::component::IconName::Copy)
        .label("Copy link")
        .on_click(move |_, _, cx: &mut App| {
            cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
        });
    if prominent { button.primary() } else { button }
}
