//! The sidebar's first row, your ember: the assistant's little picture and its name. A click
//! (or Return or Space) opens the main room, lit while it is open, with a dot while it has
//! something unread. Its menu (On duty, what it knows about the user, Settings) opens on a
//! right-click and from the ellipsis inside the row, which shows on hover and keyboard focus.
//! Off duty the picture is dimmed and grey; while it works a soft glow breathes around it
//! (still under Reduce Motion). A second line says Off duty, what it is doing or what is wrong,
//! or that a new answer waits; otherwise the row is one line high.

use std::time::Duration;

use emberhut_api::ids::RoomId;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, BoxShadow, Entity, FontWeight, Hsla,
    TestSupportExt as _, Window, div, img, pulsating_between, px,
};

use super::chat::reading::status_color;
use super::controls::tip_for;
use super::palette::Pal;
use super::routes;
use super::sidebar::row;
use super::widgets::dot;
use super::window::{MainWindow, Room0};
use crate::assets::FACE;
use crate::motion;
use crate::store::settings::SettingsPane;
use crate::store::status::{StatusLine, Tone};
use crate::store::{Page, Store};
use crate::ui::a11y;

/// What the sidebar calls the assistant.
pub(crate) const NAME: &str = "Your ember";

/// The hover group of the row, which shows its menu button.
const GROUP: &str = "ember-row";

/// The picture's width and height.
const PICTURE: f32 = 20.;

/// How long one breath of the working glow takes, in and out.
const BREATH: Duration = Duration::from_secs(2);

/// What the row's second line says.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Note {
    /// Paused: `Off duty`.
    OffDuty,
    /// What it is doing, or what is wrong (offline, out of quota): its status line.
    Status(StatusLine),
    /// The main room has an answer the user hasn't read: `New answer`.
    New,
}

impl Note {
    /// The second line of an assistant `on_duty` (or not) and `working` (or not), whose status
    /// line is `status`, with an `unread` main room (or not). None while it is ready with
    /// nothing new.
    fn of(on_duty: bool, working: bool, status: StatusLine, unread: bool) -> Option<Self> {
        if !on_duty {
            Some(Self::OffDuty)
        } else if working || matches!(status.tone, Tone::Warn | Tone::Danger) {
            Some(Self::Status(status))
        } else if unread {
            Some(Self::New)
        } else {
            None
        }
    }

    /// Its words.
    fn text(&self) -> &str {
        match self {
            Self::OffDuty => "Off duty",
            Self::Status(status) => &status.text,
            Self::New => "New answer",
        }
    }

    /// Its colour: a status line's by its tone, the accent for a new answer.
    fn color(&self, p: &Pal) -> Hsla {
        match self {
            Self::OffDuty => p.faint,
            Self::Status(status) => match status.tone {
                Tone::Quiet => p.faint,
                tone => status_color(tone, p),
            },
            Self::New => p.accent,
        }
    }
}

/// The row, with its right-click menu.
pub(crate) fn row_with_menu(
    window: &MainWindow,
    p: &Pal,
    cx: &Context<'_, MainWindow>,
) -> AnyElement {
    let store = window.store.read(cx);
    let main = store.switcher().main();
    let id = main.map_or(RoomId::MAIN, |room| room.id);
    let unread = main.is_some_and(|room| room.unread > 0);
    let on_duty = store.is_on_duty();
    let working = on_duty && store.is_working();
    let status = store.status();
    let help = format!(
        "Opens your ember's chat · {}\nOn duty: when off, your ember does nothing and wakes for \
         nothing; it still answers you in the chat.",
        status.text
    );
    let note = Note::of(on_duty, working, status, unread);
    let spoken = a11y::line(&[
        NAME,
        note.as_ref().map_or("", Note::text),
        if unread && note != Some(Note::New) {
            "new answer"
        } else {
            ""
        },
    ]);
    let text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(13.5))
                .truncate()
                .child(NAME),
        )
        .when_some(note, |d, note| {
            d.child(
                div()
                    .id("ember-note")
                    .text_size(px(11.))
                    .text_color(note.color(p))
                    .truncate()
                    .child(note.text().to_owned())
                    .test_support(),
            )
        });
    let row = tip_for(row("ember", store.room() == id, p), help, &Room0)
        .group(GROUP)
        .aria_label(spoken)
        .h_auto()
        .min_h(px(28.))
        .py(px(3.))
        .child(picture(on_duty, working, p, motion::reduced(cx)))
        .child(text)
        .when(unread, |d| d.child(dot(p.accent, 6.)))
        .child(menu_button(&window.store, on_duty, p))
        .on_click(cx.listener(move |this, _, window, cx| this.open(Page::Room(id), window, cx)))
        .test_support();
    div()
        .id("ember-menu-area")
        .pt(px(8.))
        .pb(px(4.))
        .child(row)
        .context_menu(menu(&window.store, on_duty))
        .into_any_element()
}

/// The little ember: dimmed off duty; while it works, a glow around it that breathes, or holds
/// still when the system asks for `reduced` motion.
fn picture(on_duty: bool, working: bool, p: &Pal, reduced: bool) -> impl IntoElement {
    div()
        .relative()
        .flex_none()
        .size(px(PICTURE))
        .when(working, |d| d.child(glow(p, reduced)))
        .child(
            img(FACE)
                .size_full()
                .when(!on_duty, |picture| picture.grayscale(true).opacity(0.5)),
        )
}

/// The working glow: a soft halo in the palette's glow colour, under the picture.
fn glow(p: &Pal, reduced: bool) -> AnyElement {
    let halo = div().absolute().inset_0().rounded_full().shadow(vec![
        BoxShadow::new(px(0.), px(0.), p.glow)
            .blur_radius(px(8.))
            .spread_radius(px(2.)),
    ]);
    let halo = if reduced {
        halo.into_any_element()
    } else {
        halo.with_animation(
            "ember-breath",
            Animation::new(BREATH)
                .repeat()
                .with_easing(pulsating_between(0.55, 1.)),
            Styled::opacity,
        )
        .into_any_element()
    };
    div()
        .id("ember-glow")
        .absolute()
        .inset_0()
        .child(halo)
        .test_support()
        .into_any_element()
}

/// The ellipsis inside the row that opens its menu: hidden until the row is under the pointer
/// or the button has the keyboard.
fn menu_button(store: &Entity<Store>, on_duty: bool, p: &Pal) -> impl IntoElement {
    Button::new("ember-menu")
        .icon(
            Icon::new(AssetIcon::Ellipsis)
                .size(px(13.))
                .text_color(p.muted),
        )
        .ghost()
        .xsmall()
        .tooltip("Your ember's menu")
        .accessibility_label("Your ember's menu")
        .opacity(0.)
        .group_hover(GROUP, |style| style.opacity(1.))
        .focus_visible(|style| style.opacity(1.))
        .dropdown_menu(menu(store, on_duty))
}

/// The menu: the on-duty switch, what the assistant knows about the user, and Settings.
fn menu(
    store: &Entity<Store>,
    on_duty: bool,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<'_, PopupMenu>) -> PopupMenu + 'static {
    let store = store.clone();
    move |menu, _, _| {
        let duty = store.clone();
        menu.item(
            PopupMenuItem::new("On duty")
                .checked(on_duty)
                .on_click(move |_, _, cx| {
                    duty.update(cx, |store, cx| store.set_on_duty(!on_duty, cx));
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new("What your ember knows about you…").on_click(|_, window, cx| {
                routes::open_settings(Some(SettingsPane::AboutYou), window, cx);
            }),
        )
        .item(PopupMenuItem::new("Settings…").on_click(|_, window, cx| {
            routes::open_settings(None, window, cx);
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{ElementId, TestAppContext, VisualTestContext, size};
    use serde_json::{Value, json};

    use super::*;
    use crate::backend::fake::FakeBackend;
    use crate::fixtures::{room, state};
    use crate::ui::report::ReportSize;

    /// A window over a store whose state has `extra`'s fields, Main (with `main`'s fields) and
    /// room 2.
    fn open(
        main: Value,
        extra: Value,
        cx: &mut TestAppContext,
    ) -> (Entity<Store>, Entity<MainWindow>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
        });
        let backend = Arc::new(FakeBackend::new([])) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(backend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            MainWindow::new(store.clone(), ReportSize::default(), window, cx)
        });
        cx.simulate_resize(size(px(1260.), px(800.)));
        let mut fields = json!({ "rooms": [room(1, main), room(2, json!({}))] });
        if let (Value::Object(fields), Value::Object(extra)) = (&mut fields, extra) {
            fields.extend(extra);
        }
        store.update(cx, |store, cx| {
            store.take_state_for_tests(state(fields), cx);
        });
        cx.run_until_parked();
        (store, view, cx)
    }

    /// The row as a screen reader hears it, and whether it shows its second line and its glow.
    fn shown(cx: &mut VisualTestContext) -> (String, bool, bool) {
        cx.update(|window, cx| {
            window.render_frame(cx);
            (
                window.find("ember").label().unwrap_or_default().to_owned(),
                window.try_find("ember-note").is_some(),
                window.try_find("ember-glow").is_some(),
            )
        })
    }

    /// The labels of the open menu's items, top to bottom.
    fn menu_items(cx: &mut VisualTestContext) -> Vec<String> {
        cx.update(|window, cx| {
            window.render_frame(cx);
            let menu = window.within("popup-menu");
            (0_usize..8)
                .filter_map(|ix| menu.try_find(ElementId::from(ix)))
                .filter_map(|item| item.label().map(str::to_owned))
                .collect()
        })
    }

    #[gpui_kit::test]
    fn the_row_opens_the_main_room_in_place_of_the_chat_row(cx: &mut TestAppContext) {
        let (store, view, cx) = open(json!({}), json!({}), cx);
        view.update_in(cx, |main, window, cx| {
            main.open(Page::Room(RoomId::new(2)), window, cx);
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert_ne!(window.find("ember").selected(), Some(true));
            assert!(window.try_find("chat").is_none(), "no Chat row");
            assert!(window.try_find("assistant-menu").is_none(), "no header");
            window.click("ember", cx);
        });
        cx.run_until_parked();
        assert_eq!(store.read_with(cx, |store, _| store.room()), RoomId::MAIN);
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("ember").selected(), Some(true), "lit");
        });
    }

    #[gpui_kit::test]
    fn a_right_click_opens_the_menu_with_on_duty(cx: &mut TestAppContext) {
        let (store, _, cx) = open(json!({}), json!({}), cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_none());
            window.right_click("ember", cx);
        });
        assert_eq!(
            menu_items(cx),
            ["On duty", "What your ember knows about you…", "Settings…"]
        );
        cx.update(|window, cx| window.within("popup-menu").click(0_usize, cx));
        cx.run_until_parked();
        assert!(
            !store.read_with(cx, |store, _| store.is_on_duty()),
            "switched off"
        );
    }

    #[gpui_kit::test]
    fn the_ellipsis_shows_on_hover_and_opens_the_menu_not_the_room(cx: &mut TestAppContext) {
        let (store, view, cx) = open(json!({}), json!({}), cx);
        view.update_in(cx, |main, window, cx| {
            main.open(Page::Room(RoomId::new(2)), window, cx);
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(!window.find("ember-menu").visible(), "hidden at rest");
            window.hover("ember", cx);
            window.render_frame(cx);
            assert!(window.find("ember-menu").visible(), "shown on hover");
            window.click("ember-menu", cx);
        });
        assert_eq!(menu_items(cx).first().map(String::as_str), Some("On duty"));
        assert_eq!(store.read_with(cx, |store, _| store.room()), RoomId::new(2));
    }

    #[gpui_kit::test]
    fn the_ellipsis_shows_when_the_keyboard_reaches_it(cx: &mut TestAppContext) {
        let (_, _, cx) = open(json!({}), json!({}), cx);
        // The test window has no root to bind Tab: a key press makes the input the keyboard's,
        // then focus moves on as Tab would.
        let reached = cx.update(|window, cx| {
            (0..60).any(|_| {
                window.press("shift", cx);
                window.focus_next(cx);
                window.render_frame(cx);
                window.find("ember-menu").visible()
            })
        });
        assert!(reached, "Tab reaches the ellipsis and shows it");
    }

    #[gpui_kit::test]
    fn ready_the_row_is_one_line_without_a_glow(cx: &mut TestAppContext) {
        let (_, _, cx) = open(json!({}), json!({}), cx);
        assert_eq!(shown(cx), (NAME.to_owned(), false, false));
    }

    #[gpui_kit::test]
    fn working_the_row_says_so_and_glows(cx: &mut TestAppContext) {
        let (_, _, cx) = open(json!({}), json!({"busy": true}), cx);
        assert_eq!(shown(cx), ("Your ember, Working…".to_owned(), true, true));
    }

    #[gpui_kit::test]
    fn off_duty_the_row_says_so_without_a_glow(cx: &mut TestAppContext) {
        let (_, _, cx) = open(json!({}), json!({"on": false, "busy": true}), cx);
        assert_eq!(shown(cx), ("Your ember, Off duty".to_owned(), true, false));
    }

    #[gpui_kit::test]
    fn a_new_answer_shows_on_the_second_line(cx: &mut TestAppContext) {
        let (_, _, cx) = open(json!({"unread": 1}), json!({}), cx);
        assert_eq!(
            shown(cx),
            ("Your ember, New answer".to_owned(), true, false)
        );
    }

    #[test]
    fn the_second_line_says_off_duty_then_the_status_then_a_new_answer() {
        let line = |text: &str, tone| StatusLine {
            text: text.to_owned(),
            tone,
        };
        let watching = line("Watching", Tone::Quiet);
        assert_eq!(Note::of(true, false, watching.clone(), false), None);
        assert_eq!(
            Note::of(true, false, watching.clone(), true),
            Some(Note::New)
        );
        assert_eq!(Note::of(false, false, watching, true), Some(Note::OffDuty));
        let working = line("Working…", Tone::Working);
        assert_eq!(
            Note::of(true, true, working.clone(), true),
            Some(Note::Status(working))
        );
        let offline = line("Offline", Tone::Warn);
        assert_eq!(
            Note::of(true, false, offline.clone(), false),
            Some(Note::Status(offline)),
            "what is wrong shows"
        );
    }
}
