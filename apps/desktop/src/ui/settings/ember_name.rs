//! The ember's name as the title of Your ember's hero, renamed in place: a pencil shows beside
//! it on hover and keyboard focus, and a click (or Return) turns it into a field holding the
//! name. Return, or leaving the field, keeps what is typed (blank: back to "Your ember", no
//! error); Esc puts it back. A brief "Saved" check follows a rename the server kept. The field
//! takes 24 characters (an emoji is one), counted from 20; a name the server refuses goes back,
//! with the reason under it. "Use the default name" shows while the name is the user's own.

use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, Entity, FocusHandle, FontWeight, MouseButton, SharedString, Stateful,
    TestSupportExt as _, Window, div, px,
};

use super::parts::{note, problem_line};
use crate::store::Store;
use crate::ui::a11y::Accessible as _;
use crate::ui::controls::tab_stop;
use crate::ui::palette::Pal;
use crate::ui::window::{CloseReport, RestoreReport};
use emberhut_api::ember_name::{DEFAULT_NAME, MAX_CHARS, fit, name_length};

/// The title, for tests to find.
pub(crate) const NAME_TITLE: &str = "ember-name-title";
/// The field, for tests to find.
pub(crate) const NAME_FIELD: &str = "ember-name-field";
/// The link back to the default name, for tests to find.
pub(crate) const NAME_RESET: &str = "ember-name-reset";
/// The "Saved" check, for tests to find.
pub(crate) const NAME_SAVED: &str = "ember-name-saved";

/// What the name is for, said on the title's tip and under the field.
pub(crate) const HINT: &str = "Shows in the sidebar, chats, notifications and on your phone.";

/// The length from which the field counts what is typed.
const COUNT_FROM: usize = 20;

/// How long the "Saved" check shows.
const SAVED_FOR: Duration = Duration::from_millis(1500);

/// The hover group of the title, which shows its pencil.
const GROUP: &str = "ember-name";

/// The ember's name, renamed in place.
pub(crate) struct NameTitle {
    store: Entity<Store>,
    pub(super) input: Entity<InputState>,
    /// The title's focus: given back after Return or Esc, and taken when the Name setting is
    /// opened from search.
    pub(super) title: FocusHandle,
    /// Whether the field shows in place of the title.
    pub(super) editing: bool,
    /// Whether the title takes the focus once it is drawn (its setting was opened).
    pub(super) wants_focus: bool,
    /// The last kept rename whose check has had its time.
    check_timed: Option<Instant>,
}

impl NameTitle {
    /// The title over `store`.
    pub(crate) fn new(
        store: Entity<Store>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(DEFAULT_NAME));
        cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.finish(true, true, window, cx),
                InputEvent::Blur => this.finish(true, false, window, cx),
                InputEvent::Change => this.typed(window, cx),
                InputEvent::Focus => {}
            },
        )
        .detach();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            input,
            title: cx.focus_handle(),
            editing: false,
            wants_focus: false,
            check_timed: None,
        }
    }

    /// Turns the title into the field, holding the user's name for the ember (empty for the
    /// default) all selected.
    pub(super) fn edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let name = self
            .store
            .read(cx)
            .ember_custom_name()
            .unwrap_or_default()
            .to_owned();
        self.editing = true;
        self.input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// The field goes back to the title: the typed name kept when `keep` (else the field's
    /// words are dropped), the title focused again when `refocus`.
    fn finish(
        &mut self,
        keep: bool,
        refocus: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.editing {
            return;
        }
        self.editing = false;
        if keep {
            let typed = self.input.read(cx).value().to_string();
            let same = self.store.read(cx).ember_custom_name().unwrap_or_default() == typed.trim();
            if !same {
                self.store
                    .update(cx, |store, cx| store.rename_ember(&typed, cx));
            }
        }
        if refocus {
            window.focus(&self.title, cx);
        }
        cx.notify();
    }

    /// Back to "Your ember", whether or not the field shows.
    pub(super) fn reset(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Before the field's blur, so leaving it doesn't keep its words over this.
        self.editing = false;
        self.store
            .update(cx, |store, cx| store.rename_ember("", cx));
        window.focus(&self.title, cx);
        cx.notify();
    }

    /// The field changed: past 24 characters what was typed or pasted is cut there, and once
    /// the user types the last refusal's reason goes.
    fn typed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let typed = self.input.read(cx).value().to_string();
        if name_length(&typed) > MAX_CHARS {
            let cut = fit(&typed).to_owned();
            self.input
                .update(cx, |input, cx| input.set_value(cut, window, cx));
        }
        if self.store.read(cx).ember_name_problem().is_some() {
            self.store.update(cx, Store::dismiss_ember_name_problem);
        }
        cx.notify();
    }

    /// Whether the check shows now, after the server kept a rename; it goes by itself.
    fn shows_check(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(saved) = self.store.read(cx).ember_name_saved_at() else {
            return false;
        };
        let left = SAVED_FOR.saturating_sub(saved.elapsed());
        if left.is_zero() {
            return false;
        }
        if self.check_timed != Some(saved) {
            self.check_timed = Some(saved);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(left).await;
                if let Err(error) = this.update(cx, |_, cx| cx.notify()) {
                    log::debug!("Settings closed before the name's check went: {error}");
                }
            })
            .detach();
        }
        true
    }

    /// The title: the name (up to two lines) and, on hover or while `focused`, its pencil.
    fn title(&self, name: &str, focused: bool, p: &Pal) -> Stateful<Div> {
        let pencil = p.muted;
        let tip = SharedString::from(format!("Rename · {HINT}"));
        tab_stop(
            div()
                .id(NAME_TITLE)
                .track_focus(&self.title)
                .group(GROUP)
                .button_named(format!("Rename, now {name}"))
                .flex()
                .items_start()
                .gap(px(6.))
                .min_w_0()
                .px(px(4.))
                .mx(px(-4.))
                .rounded(px(6.))
                .border_1()
                .cursor_pointer()
                .hover(|d| d.bg(p.hover))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
            p,
        )
        .child(
            div()
                .min_w_0()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(15.))
                .line_clamp(2)
                .text_ellipsis()
                .child(name.to_owned()),
        )
        .child(
            div()
                .flex_none()
                .pt(px(3.))
                .opacity(0.)
                .group_hover(GROUP, |style| style.opacity(1.))
                .when(focused, |d| d.opacity(1.))
                .child(Icon::new(IconName::Pencil).size(px(13.)).text_color(pencil)),
        )
    }

    /// The field in place of the title, its count from 20 characters on, and what the name
    /// is for under it.
    fn field(&self, p: &Pal, cx: &App) -> AnyElement {
        let count = name_length(&self.input.read(cx).value());
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id(NAME_FIELD)
                            .flex_1()
                            .min_w_0()
                            .max_w(px(320.))
                            .child(Input::new(&self.input).small())
                            .test_support(),
                    )
                    .when(count >= COUNT_FROM, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(px(11.5))
                                .text_color(if count >= MAX_CHARS { p.warn } else { p.muted })
                                .child(format!("{count}/{MAX_CHARS}")),
                        )
                    }),
            )
            .child(note(p, HINT))
            .into_any_element()
    }

    /// The link back to the default name, while the name is the user's own. A press goes
    /// before the field's blur, so the field's words aren't kept over it.
    pub(super) fn reset_link(&self, p: &Pal, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        self.store.read(cx).ember_custom_name()?;
        let link = div()
            .id(NAME_RESET)
            .button_named("Use the default name")
            .text_size(px(12.))
            .text_color(p.muted)
            .cursor_pointer()
            .border_1()
            .rounded(px(4.))
            .hover(|d| d.text_color(p.text).underline())
            .child("Use the default name")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.reset(window, cx);
                }),
            )
            // Return or Space on it, from the keyboard.
            .on_click(cx.listener(|this, _, window, cx| {
                if this.store.read(cx).ember_custom_name().is_some() {
                    this.reset(window, cx);
                }
            }));
        Some(
            div()
                .flex()
                .child(tab_stop(link, p).test_support())
                .into_any_element(),
        )
    }
}

/// The "Saved" check after a kept rename.
fn saved(p: &Pal) -> impl IntoElement {
    div()
        .id(NAME_SAVED)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .pt(px(3.))
        .text_size(px(12.))
        .text_color(p.success)
        .child(
            Icon::new(IconName::Check)
                .size(px(12.))
                .text_color(p.success),
        )
        .child("Saved")
        .test_support()
}

impl Render for NameTitle {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p: Pal = crate::ui::palette::pal(cx);
        let store = self.store.read(cx);
        let name = store.ember_name().to_owned();
        let problem = store.ember_name_problem().map(str::to_owned);
        if std::mem::take(&mut self.wants_focus) && !self.editing {
            // Once this frame has drawn the title: a handle not drawn yet can't keep the focus.
            let title = self.title.clone();
            window.defer(cx, move |window, cx| window.focus(&title, cx));
        }
        let focused = self.title.is_focused(window);
        let check = self.shows_check(cx);
        let head = if self.editing {
            self.field(&p, cx)
        } else {
            let title = self.title(&name, focused, &p);
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                .min_w_0()
                .child(
                    title
                        .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx)))
                        .test_support(),
                )
                .when(check, |d| d.child(saved(&p)))
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            // Esc puts the name back; the window's Esc (the report's) waits while it does.
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.editing {
                    cx.stop_propagation();
                    this.finish(false, true, window, cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &RestoreReport, window, cx| {
                if this.editing {
                    cx.stop_propagation();
                    this.finish(false, true, window, cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &CloseReport, window, cx| {
                if this.editing {
                    cx.stop_propagation();
                    this.finish(false, true, window, cx);
                }
            }))
            .child(head)
            .children(problem.map(|problem| problem_line(&p, problem)))
    }
}
