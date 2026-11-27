//! Settings: one page of rows and sections (Appearance, the MCP servers and skills with their
//! switches, the data sources, where the assistant works, what it knows about the user, sign-in,
//! the devices, the server and the archived rooms), and the pages its rows open, each in its own
//! module.

mod about;
mod account;
mod appearance;
mod chrome;
mod chrome_profile;
mod chrome_sheets;
pub(crate) mod environment;
mod filter_bar;
pub(crate) mod parts;
mod places;
mod share;
mod switches;
mod updates;
mod usage;

use emberhut_identity::PairingLink;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, Entity, FocusHandle, Window, div, px};

use self::about::AboutPane;
use self::appearance::AppearancePane;
use self::chrome::ChromePane;
use self::parts::{nav_row, page_body, problem_line, section};
use super::column;
use super::pairing::PairingForm;
use super::palette::{Pal, category_color, pal};
use super::qr::Qr;
use super::settings_more;
use super::widgets::{group, page_head};
use crate::brand::APP_NAME;
use crate::store::devices::NewCode;
use crate::store::settings::SettingsPane;
use crate::store::settings::appearance::summary;
use crate::store::settings::chrome::{self as chrome_words, Health};
use crate::store::working::settings_filter::SettingsFilter;
use crate::store::{Metrics, Page, Store};

/// The settings page.
pub(crate) struct SettingsView {
    store: Entity<Store>,
    pairing: Entity<PairingForm>,
    appearance: Entity<AppearancePane>,
    about: Entity<AboutPane>,
    chrome: Entity<ChromePane>,
    /// The account's sign-in sheet's focus.
    account_sheet: FocusHandle,
    /// The pairing link last drawn as a QR code, for which display scale, and its picture.
    qr: Option<((String, u32), Qr)>,
}

impl SettingsView {
    /// The page over `store`, with the pairing form it shares with the sign-in screen.
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
        Self {
            store,
            pairing,
            appearance,
            about,
            chrome,
            account_sheet: cx.focus_handle(),
            qr: None,
        }
    }

    /// A row that opens one of Settings' pages, over Settings in the window's column (Back
    /// returns here).
    fn opens(
        pane: SettingsPane,
    ) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static {
        move |_, window, cx| column::push(Page::SettingsPane(pane), window, cx)
    }

    /// One of Settings' pages, as the column shows it over Settings (its bar goes Back).
    pub(crate) fn pane(&self, pane: SettingsPane, p: &Pal) -> Div {
        let (title, body) = match pane {
            SettingsPane::Appearance => ("Appearance", self.appearance.clone().into_any_element()),
            SettingsPane::ChromeProfiles => {
                ("Chrome profiles", self.chrome.clone().into_any_element())
            }
            SettingsPane::AboutYou => ("About you", self.about.clone().into_any_element()),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(page_head(p, title, None))
            .child(div().flex_1().min_h_0().child(body))
    }

    /// The QR code of the pairing link made for another device, for a display of `scale`,
    /// drawn once a link and scale. Only a link for the code shown is drawn.
    fn pairing_qr(&mut self, scale: f32, cx: &mut Context<'_, Self>) -> Option<Qr> {
        let link = match self.store.read(cx).new_code() {
            Some(NewCode::Made(code)) => code.link.clone().filter(|link| {
                PairingLink::parse(link).is_ok_and(|read| same_code(read.code(), &code.code))
            }),
            _ => None,
        };
        let wanted = link.map(|link| (link, scale.to_bits()));
        if self.qr.as_ref().map(|(drawn, _)| drawn) != wanted.as_ref() {
            if let Some((_, old)) = self.qr.take() {
                cx.drop_image(old.picture, None);
            }
            self.qr = wanted.and_then(|(link, bits)| {
                super::qr::qr(&link, scale).map(|drawn| ((link, bits), drawn))
            });
        }
        self.qr.as_ref().map(|(_, drawn)| drawn.clone())
    }

    /// The main page's sections.
    fn main_page(&mut self, p: &Pal, scale: f32, cx: &mut Context<'_, Self>) -> Div {
        let qr = self.pairing_qr(scale, cx);
        let devices = settings_more::devices(&self.store, qr, p, cx);
        let archived_open = self.store.read(cx).archived_open();
        let archived = settings_more::archived(&self.store, archived_open, p, cx);
        let store = self.store.read(cx);
        let theme = summary(store.theme().page());
        let profiles = chrome_words::summary(store.chrome_profiles());
        let (machines, machine_names) = settings_more::machines_row(store);
        let server = settings_more::server_rows(store, p);
        let action_problem = store.action_problem().map(str::to_owned);
        let metrics = metrics_line(store.metrics());
        let filter = store.settings_filter();
        let (chrome_health, machines_health) = (profiles.health, machines.1);
        let appearance = nav_row(
            "appearance-row",
            p,
            (IconName::Palette, p.accent),
            "Appearance",
            (theme, Health::Off),
            "",
        )
        .on_click(Self::opens(SettingsPane::Appearance));
        let chrome = nav_row(
            "chrome-profiles-row",
            p,
            (IconName::Globe, p.accent),
            "Chrome profiles",
            (profiles.status, profiles.health),
            &profiles.detail,
        )
        .on_click(Self::opens(SettingsPane::ChromeProfiles));
        let machines = nav_row(
            "machines-row",
            p,
            (IconName::Cpu, category_color(5)),
            "Machines",
            machines,
            &machine_names,
        )
        // Over Settings in the column: Back returns here.
        .on_click(move |_, window, cx| column::push(Page::Machines, window, cx));
        let about = nav_row(
            "about-you-row",
            p,
            (IconName::Contact, category_color(0)),
            format!("What {APP_NAME} knows about you"),
            ("Facts and rules on trial".to_owned(), Health::Off),
            "",
        )
        .on_click(Self::opens(SettingsPane::AboutYou));
        let mut parts: Vec<AnyElement> = Vec::new();
        if filter.is_empty() {
            parts.push(updates::section_for(&self.store, p, cx).into_any_element());
            parts.push(
                section(p, "", None, vec![appearance.into_any_element()], None).into_any_element(),
            );
        }
        let chrome = places::OtherRow {
            row: chrome.into_any_element(),
            health: chrome_health,
        };
        let machines = places::OtherRow {
            row: machines.into_any_element(),
            health: machines_health,
        };
        parts.extend(self.filtered_parts(filter, chrome, machines, p, cx));
        if !filter.is_empty() {
            return div().flex().flex_col().gap(px(18.)).children(parts);
        }
        parts.push(
            section(p, "About you", None, vec![about.into_any_element()], None).into_any_element(),
        );
        parts.push(usage::card(&self.store, p, cx).into_any_element());
        parts.push(
            group(p, "SIGN IN")
                .child(self.pairing.clone())
                .into_any_element(),
        );
        parts.push(devices.into_any_element());
        if let Some(signed) = crate::account::installed(cx) {
            parts.push(account::section(&signed, p, cx).into_any_element());
        }
        parts.push(
            group(p, "SERVER")
                .children(server)
                .child(div().text_size(px(12.)).text_color(p.muted).child(metrics))
                .into_any_element(),
        );
        parts.push(archived.into_any_element());
        if let Some(problem) = action_problem {
            parts.push(problem_line(p, problem).into_any_element());
        }
        div().flex().flex_col().gap(px(18.)).children(parts)
    }
}

impl SettingsView {
    /// The parts the filter narrows: the MCP servers and skills, the data sources and where
    /// the assistant works (with the Chrome profiles' and the Machines rows), and why a list
    /// didn't refresh.
    fn filtered_parts(
        &self,
        filter: SettingsFilter,
        chrome: places::OtherRow,
        machines: places::OtherRow,
        p: &Pal,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let mut parts = switches::sections(&self.store, filter, p, cx);
        parts.extend(
            places::sources(&self.store, filter, chrome, p, cx).map(IntoElement::into_any_element),
        );
        parts.extend(
            places::workplaces(&self.store, filter, machines, p, cx)
                .map(IntoElement::into_any_element),
        );
        let store = self.store.read(cx);
        let places_problem = store
            .environments()
            .page()
            .and(store.environments().problem())
            .map(str::to_owned);
        let problems = [switches::problem(&self.store, cx), places_problem];
        parts.extend(
            problems
                .into_iter()
                .flatten()
                .map(|problem| problem_line(p, problem).into_any_element()),
        );
        parts
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let p = pal(cx);
        let sheet = crate::account::installed(cx)
            .and_then(|signed| account::sheet_for(&signed, &self.account_sheet, &p, cx));
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(page_head(&p, "Settings", None))
            .child(filter_bar::bar(&self.store, &p, cx))
            .child(page_body(
                "settings-scroll",
                self.main_page(&p, window.scale_factor(), cx),
            ))
            .children(sheet)
    }
}

/// Whether two pairing codes are the same as the server reads them: case, spaces and the dash
/// don't matter.
fn same_code(a: &str, b: &str) -> bool {
    let plain = |code: &str| {
        code.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>()
    };
    plain(a) == plain(b)
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
mod tests {
    use std::sync::Arc;

    use emberhut_api::chrome::ChromeAction;
    use emberhut_api::facts::{FactCategory, FactsPage};
    use emberhut_api::ids::{ChromeProfileId, FactId};
    use emberhut_api::settings::{FactChange, SettingsAction};
    use emberhut_api::switches::SwitchAction;
    use gpui_kit::component::input::InputState;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{TestAppContext, VisualTestContext};
    use serde_json::json;

    use super::*;
    use crate::backend::Error;
    use crate::backend::fake::{Call, FakeBackend};
    use crate::store::settings::chrome::ChromeSheet;

    /// What a test window shows: Settings, or one of its pages as the column shows it.
    struct Shown {
        view: Entity<SettingsView>,
        pane: Option<SettingsPane>,
    }

    impl Render for Shown {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            match self.pane {
                None => self.view.clone().into_any_element(),
                Some(pane) => self.view.read(cx).pane(pane, &pal(cx)).into_any_element(),
            }
        }
    }

    /// Settings over a fake backend, in a headless window.
    fn open_settings(
        cx: &mut TestAppContext,
    ) -> (Arc<FakeBackend>, Entity<Store>, VisualTestContext) {
        let (backend, store, _, cx) = open_settings_view(cx, None);
        (backend, store, cx)
    }

    /// Settings (or one of its pages, `pane`) over a fake backend, in a headless window, and
    /// its view.
    fn open_settings_view(
        cx: &mut TestAppContext,
        pane: Option<SettingsPane>,
    ) -> (
        Arc<FakeBackend>,
        Entity<Store>,
        Entity<SettingsView>,
        VisualTestContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Pal::new(&crate::theme::built_in(), false));
            parts::bind_keys(cx);
        });
        let backend = Arc::new(FakeBackend::new([]));
        let shared = Arc::clone(&backend) as Arc<dyn crate::backend::Backend>;
        let store = cx.new(|_| Store::for_tests(shared));
        let window = cx.add_window(|window, cx| {
            let pairing = cx.new(|cx| PairingForm::new(store.clone(), window, cx));
            let view = cx.new(|cx| SettingsView::new(store.clone(), pairing, window, cx));
            Shown { view, pane }
        });
        let view = window
            .root(cx)
            .unwrap()
            .read_with(cx, |shown, _| shown.view.clone());
        let cx = VisualTestContext::from_window(window.into(), cx);
        (backend, store, view, cx)
    }

    /// Empties `field`, gives it the focus and types `text` in it, as the user would.
    fn type_in(field: &Entity<InputState>, text: &str, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            field.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            window.render_frame(cx);
        });
        cx.simulate_input(text);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn a_skills_switch_sends_its_change(cx: &mut TestAppContext) {
        let (backend, store, mut cx) = open_settings(cx);
        let page = serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z", "live": true,
            "mcp": [], "skills": [{"name": "notes", "description": "Takes notes.",
            "source": "yours", "on": true}]}))
        .unwrap();
        backend.replies.push_switches(Ok(page));
        store.update(&mut cx, |store, cx| {
            store.switch(SwitchAction::Reconnect { name: "x".into() }, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("skill:notes-switch", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            backend.calls().last(),
            Some(&Call::Switch(SwitchAction::Skill {
                name: "notes".into(),
                on: false
            }))
        );
    }

    #[gpui_kit::test]
    fn return_is_done_on_a_chrome_profiles_sheet(cx: &mut TestAppContext) {
        let (_backend, store, _, mut cx) =
            open_settings_view(cx, Some(SettingsPane::ChromeProfiles));
        store.update(&mut cx, |store, cx| {
            store.open_chrome_profile(ChromeProfileId::new(2), cx);
        });
        cx.run_until_parked();
        cx.update(Window::render_frame);
        assert_eq!(
            store.read_with(&cx, |store, _| store.chrome().sheet().cloned()),
            Some(ChromeSheet::Profile(ChromeProfileId::new(2)))
        );
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(
            store.read_with(&cx, |store, _| store.chrome().sheet().cloned()),
            None
        );
    }

    #[gpui_kit::test]
    fn return_in_the_name_field_renames_and_a_refusal_shows_on_the_sheet(cx: &mut TestAppContext) {
        let (backend, store, view, mut cx) =
            open_settings_view(cx, Some(SettingsPane::ChromeProfiles));
        let id = ChromeProfileId::new(2);
        store.update(&mut cx, |store, cx| {
            store.put_state_for_tests(crate::fixtures::state(json!({"chrome": [{"id": 2,
                "name": "work", "label": "Work", "connected": true,
                "created": "2026-11-01T09:00:00Z"}]})));
            store.open_chrome_profile(id, cx);
        });
        cx.run_until_parked();
        cx.update(Window::render_frame);
        backend.replies.push_chrome(Err(Error::Refused {
            status: Some(400),
            message: Some("There's already a Chrome profile called Laptop".into()),
        }));
        let field = view.read_with(&cx, |view, cx| view.chrome.read(cx).rename.clone());
        type_in(&field, "Laptop", &mut cx);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(Window::render_frame);
        assert_eq!(
            backend.calls().last(),
            Some(&Call::Chrome(ChromeAction::Rename {
                id,
                label: "Laptop".into()
            }))
        );
        store.read_with(&cx, |store, _| {
            assert_eq!(
                store.chrome().sheet(),
                Some(&ChromeSheet::Profile(id)),
                "the sheet stays open"
            );
            assert_eq!(
                store.chrome().problem(),
                Some("There's already a Chrome profile called Laptop")
            );
        });
        let kept = field.read_with(&cx, |input, _| input.value().to_string());
        assert_eq!(kept, "Work", "the field goes back to the name kept");
    }

    #[gpui_kit::test]
    fn return_in_fix_a_fact_saves_the_correction(cx: &mut TestAppContext) {
        let (backend, store, view, mut cx) = open_settings_view(cx, Some(SettingsPane::AboutYou));
        let page: FactsPage = serde_json::from_value(json!({"facts": [
            {"id": "work/desk", "category": "work", "title": "Desk", "text": "Works from home.",
             "source": "chat", "updated": "2026-11-20T10:00:00Z"}]}))
        .unwrap();
        backend.replies.push_facts(Ok(page));
        store.update(&mut cx, |store, cx| {
            store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::AboutYou)), cx);
        });
        cx.run_until_parked();
        let desk = FactId::new("work/desk");
        let about = view.read_with(&cx, |view, _| view.about.clone());
        cx.update(|window, cx| {
            let fact = store.read(cx).facts().fact(&desk).cloned().unwrap();
            about.update(cx, |about, cx| about.edit(&fact, window, cx));
            window.render_frame(cx);
        });
        cx.run_until_parked();
        let field = about.read_with(&cx, |about, _| about.text.clone());
        type_in(&field, "Works from the office.", &mut cx);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [
                Call::Facts,
                Call::Settings(SettingsAction::FactSet(FactChange {
                    id: desk,
                    category: FactCategory::Work,
                    title: "Desk".into(),
                    text: "Works from the office.".into(),
                })),
                Call::Facts,
            ],
            "saved, then read again for what the server keeps"
        );
    }

    #[gpui_kit::test]
    fn the_usage_card_and_where_the_assistant_works_draw_and_answer(cx: &mut TestAppContext) {
        let (backend, store, mut cx) = open_settings(cx);
        let page = serde_json::from_value(json!({"at": "2026-01-01T00:00:00Z",
            "summary": {"ok": 1, "warn": 0, "down": 0},
            "groups": [{"id": "runners", "title": "Runners", "footer": null, "items": [
                {"id": "runner", "group": "runners", "name": "Runner", "icon": "server.rack",
                 "tint": "teal", "state": "ok", "status": "Online", "detail": "2 sessions",
                 "at": null, "error": null, "hint": null, "facts": [], "lists": [],
                 "actions": [], "lastError": null}]}]}))
        .unwrap();
        backend.working.push_environment_action(Ok(page));
        store.update(&mut cx, |store, cx| {
            let mut state = crate::fixtures::state(json!({
                "usage": [{"id": "a-1", "name": "Example", "scope": "any", "loggedIn": true,
                    "used5h": 80, "resets5h": null, "used7d": 20, "resets7d": null,
                    "limitedUntil": null, "checked": null, "places": [], "best": true}]}));
            state.assistant_account = Some(emberhut_api::usage::AssistantAccount {
                on: Some("a-1".to_owned()),
                pin: None,
                mode: "auto".to_owned(),
                pin_out: false,
                rooms: Vec::new(),
                waits: None,
            });
            store.put_state_for_tests(state);
            store.act_on_environment(
                emberhut_api::ids::EnvironmentId::new("runner"),
                emberhut_api::environments::EnvActionKind::Check,
                cx,
            );
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("usage-check-now", cx);
        });
        cx.run_until_parked();
        store.update(&mut cx, |store, cx| store.toggle_usage_row("a-1", cx));
        cx.update(Window::render_frame);
        assert!(store.read_with(&cx, |store, _| store.usage_card().is_open("a-1")));
        store.update(&mut cx, |store, cx| {
            let filter = SettingsFilter {
                part: Some(crate::store::working::settings_filter::SettingsPart::Machines),
                health: None,
            };
            store.set_settings_filter(filter, cx);
        });
        cx.update(Window::render_frame);
    }
}
