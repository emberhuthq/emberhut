//! Settings: one page of rows and sections (Appearance, what the ember thinks with, the MCP
//! servers and skills with their
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
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, Entity, FocusHandle, Window, div, px};

use self::about::AboutPane;
use self::appearance::AppearancePane;
use self::chrome::ChromePane;
use self::parts::{nav_row, page_body, problem_line, section};
use super::column;
use super::ember::credentials::CredentialsView;
use super::pairing::PairingForm;
use super::palette::{Pal, category_color, pal};
use super::qr::Qr;
use super::settings_more;
use super::widgets::{group, page_head};
use crate::store::credentials::{not_in_use, saved_line};
use crate::store::devices::NewCode;
use crate::store::settings::SettingsPane;
use crate::store::settings::about::TITLE as ABOUT_TITLE;
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
    credentials: Entity<CredentialsView>,
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
        // Once a provider is saved, back to Settings.
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
        Self {
            store,
            pairing,
            appearance,
            about,
            chrome,
            credentials,
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
            SettingsPane::AboutYou => (ABOUT_TITLE, self.about.clone().into_any_element()),
            SettingsPane::Credentials => (
                "What your ember thinks with",
                self.credentials.clone().into_any_element(),
            ),
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
        let thinks_with = thinks_with_row(store, p);
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
            ABOUT_TITLE,
            (
                "Facts, and lessons from your corrections".to_owned(),
                Health::Off,
            ),
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
            section(p, "Your ember", None, vec![about.into_any_element()], None).into_any_element(),
        );
        parts.push(usage::card(&self.store, p, cx).into_any_element());
        parts.push(section(p, "Your ember", None, vec![thinks_with], None).into_any_element());
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
mod tests {
    use std::sync::Arc;

    use emberhut_api::agent_provider::{
        AgentProvider, ProviderKind, ProviderModel, ProviderRequest, ProviderTest,
    };
    use emberhut_api::chrome::ChromeAction;
    use emberhut_api::facts::{FactCategory, FactsPage};
    use emberhut_api::ids::{ChromeProfileId, FactId, TrialId};
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

    /// What your ember knows about you over a fake backend that answers with `page`, drawn.
    fn open_about(
        cx: &mut TestAppContext,
        page: serde_json::Value,
    ) -> (
        Arc<FakeBackend>,
        Entity<Store>,
        Entity<SettingsView>,
        VisualTestContext,
    ) {
        let (backend, store, view, mut cx) = open_settings_view(cx, Some(SettingsPane::AboutYou));
        backend
            .replies
            .push_facts(Ok(serde_json::from_value(page).unwrap()));
        store.update(&mut cx, |store, cx| {
            store.show_in_test_column(Some(Page::SettingsPane(SettingsPane::AboutYou)), cx);
        });
        cx.run_until_parked();
        cx.update(Window::render_frame);
        (backend, store, view, cx)
    }

    /// Two facts in two categories and a lesson on trial, with its line to the user.
    fn known() -> serde_json::Value {
        json!({
            "facts": [
                {"id": "people/example", "category": "people", "title": "Example Person",
                 "text": "Runs the example team.", "source": "chat",
                 "updated": "2026-11-20T10:00:00Z", "important": true},
                {"id": "work/desk", "category": "work", "title": "Desk",
                 "text": "Works from home.", "source": "chat", "updated": "2026-11-20T10:00:00Z"}],
            "trials": [{"id": 4, "text": "Before telling the owner a file changed, say which.",
                        "at": "2026-11-20T10:00:00Z", "ends": "2026-12-04T10:00:00Z",
                        "said": "Which file was that?",
                        "lesson": "I say which file a change is in."}]})
    }

    #[gpui_kit::test]
    fn learning_from_you_is_one_folded_line_over_the_chips_and_facts(cx: &mut TestAppContext) {
        let (_backend, store, _, mut cx) = open_about(cx, known());
        let learning = cx
            .debug_bounds(about::LEARNING)
            .expect("Learning from you is drawn");
        let chips = cx.debug_bounds(about::CHIPS).expect("the chips are drawn");
        let fact = cx.debug_bounds(about::FACT_ROW).expect("a fact is drawn");
        assert!(
            learning.bottom() <= chips.top() && chips.bottom() <= fact.top(),
            "lessons, then chips, then facts: {learning:?} {chips:?} {fact:?}"
        );
        assert!(
            cx.debug_bounds(about::TRIAL_ROW).is_none(),
            "folded at first"
        );
        cx.update(|window, cx| window.click(about::LEARNING_TOGGLE, cx));
        cx.run_until_parked();
        cx.update(Window::render_frame);
        assert!(cx.debug_bounds(about::TRIAL_ROW).is_some(), "opened");
        assert!(store.read_with(&cx, |store, _| store.facts().learning_open()));
    }

    #[gpui_kit::test]
    fn keep_keeps_the_lesson_now(cx: &mut TestAppContext) {
        let (backend, store, _, mut cx) = open_about(cx, known());
        store.update(&mut cx, Store::toggle_learning);
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("trial-4-keep", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            backend.calls(),
            [
                Call::Facts,
                Call::Settings(SettingsAction::TrialKeep(TrialId::new(4))),
                Call::Facts,
            ]
        );
    }

    #[gpui_kit::test]
    fn drop_asks_once_before_it_drops(cx: &mut TestAppContext) {
        let (backend, store, _, mut cx) = open_about(cx, known());
        store.update(&mut cx, Store::toggle_learning);
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("trial-4-drop", cx);
        });
        cx.run_until_parked();
        assert_eq!(backend.calls(), [Call::Facts], "asked, not sent");
        cx.update(|window, cx| window.click("trial-4-confirm", cx));
        cx.run_until_parked();
        assert_eq!(
            backend.calls()[1],
            Call::Settings(SettingsAction::TrialDrop(TrialId::new(4)))
        );
    }

    #[gpui_kit::test]
    fn a_facts_edit_and_forget_buttons_act(cx: &mut TestAppContext) {
        let (backend, _store, view, mut cx) = open_about(cx, known());
        assert!(cx.debug_bounds(about::FACT_EDIT).is_some(), "Edit shows");
        assert!(
            cx.debug_bounds(about::FACT_FORGET).is_some(),
            "Forget shows"
        );
        cx.update(|window, cx| window.click("fact-work/desk-edit", cx));
        cx.run_until_parked();
        let about = view.read_with(&cx, |view, _| view.about.clone());
        let field = about.read_with(&cx, |about, _| about.text.clone());
        assert_eq!(
            field.read_with(&cx, |input, _| input.value().to_string()),
            "Works from home.",
            "Fix a fact opens on it"
        );
        cx.update(|window, cx| window.click("sheet-done", cx));
        cx.run_until_parked();

        cx.update(|window, cx| window.click("fact-work/desk-forget", cx));
        cx.run_until_parked();
        assert_eq!(backend.calls(), [Call::Facts], "asked, not sent");
        cx.update(|window, cx| window.click("fact-work/desk-forget-confirm", cx));
        cx.run_until_parked();
        assert_eq!(
            backend.calls()[1],
            Call::Settings(SettingsAction::FactDrop(FactId::new("work/desk")))
        );
    }

    #[gpui_kit::test]
    fn a_chip_narrows_the_list_to_its_category(cx: &mut TestAppContext) {
        let (_backend, _store, _, mut cx) = open_about(cx, known());
        let shown = |cx: &mut VisualTestContext| {
            cx.update(|window, _| {
                (
                    window.try_find("fact-people/example-edit").is_some(),
                    window.try_find("fact-work/desk-edit").is_some(),
                )
            })
        };
        assert_eq!(shown(&mut cx), (true, true), "All");
        // All, People, Work.
        cx.update(|window, cx| window.click("about-chip-2", cx));
        cx.run_until_parked();
        cx.update(Window::render_frame);
        assert_eq!(shown(&mut cx), (false, true), "Work only");
        cx.update(|window, cx| window.click("about-chip-0", cx));
        cx.run_until_parked();
        cx.update(Window::render_frame);
        assert_eq!(shown(&mut cx), (true, true), "All again");
    }

    #[gpui_kit::test]
    fn long_rules_and_facts_wrap_inside_a_narrow_window(cx: &mut TestAppContext) {
        let long = "A scan or review is done only when every finding is marked landed or \
                    superseded after a check in the code, not when its session says done. "
            .repeat(3);
        let (_backend, store, _, mut cx) = open_about(
            cx,
            json!({
                "facts": [{"id": "work/desk", "category": "work", "title": long, "text": long,
                           "source": "chat", "updated": "2026-11-20T10:00:00Z"}],
                "trials": [{"id": 4, "text": long, "at": "2026-11-20T10:00:00Z",
                            "ends": "2026-12-04T10:00:00Z", "said": long, "lesson": long}]}),
        );
        store.update(&mut cx, Store::toggle_learning);
        cx.simulate_resize(gpui_kit::size(px(560.), px(900.)));
        cx.update(Window::render_frame);
        let edge = px(560.);
        let row = cx
            .debug_bounds(about::TRIAL_ROW)
            .expect("the lesson is drawn");
        let drop = cx
            .debug_bounds(about::TRIAL_DROP)
            .expect("its Drop is drawn");
        let keep = cx
            .debug_bounds(about::TRIAL_KEEP)
            .expect("its Keep is drawn");
        let fact = cx.debug_bounds(about::FACT_ROW).expect("the fact is drawn");
        let forget = cx
            .debug_bounds(about::FACT_FORGET)
            .expect("its Forget is drawn");
        assert!(
            row.right() <= edge,
            "the lesson wraps inside the window: {row:?}"
        );
        for button in [drop, keep] {
            assert!(
                button.right() <= row.right(),
                "the button stays in its row: {button:?} in {row:?}"
            );
            assert!(button.size.width > px(0.), "it has room: {button:?}");
        }
        assert!(
            fact.right() <= edge,
            "the fact wraps inside the window: {fact:?}"
        );
        assert!(
            forget.right() <= fact.right() && forget.size.width > px(0.),
            "Forget stays in its row: {forget:?} in {fact:?}"
        );
    }

    #[gpui_kit::test]
    fn the_thinks_with_pane_tests_a_key_then_uses_it(cx: &mut TestAppContext) {
        let pane = SettingsPane::Credentials;
        let (backend, store, view, mut cx) = open_settings_view(cx, Some(pane));
        let model = |id: &str, name: Option<&str>| ProviderModel {
            id: id.to_owned(),
            name: name.map(str::to_owned),
        };
        // No local Ollama answers the look as the pane opens.
        backend
            .provider
            .push_test(Err(Error::Unreachable("connection refused".into())));
        backend.provider.push_test(Ok(ProviderTest {
            checked: true,
            models: vec![
                model("example-model-a", None),
                model("example-model-b", Some("Example B")),
            ],
        }));
        store.update(&mut cx, |store, cx| {
            store.show_in_test_column(Some(Page::SettingsPane(pane)), cx);
        });
        cx.run_until_parked();
        let mut opened = backend.calls();
        opened.sort_by_key(|call| matches!(call, Call::TestAgentProvider(_)));
        assert_eq!(
            opened,
            [Call::AgentProvider, Call::TestAgentProvider(ollama_look())],
            "read, and Ollama looked for, as it opens"
        );
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("provider-openai", cx);
        });
        cx.run_until_parked();
        let picked = |cx: &VisualTestContext| {
            store.read_with(cx, |store, _| store.credentials().picked().cloned())
        };
        assert_eq!(picked(&cx), None, "coming next: not picked");
        cx.update(|window, cx| window.click("provider-anthropic", cx));
        cx.run_until_parked();
        assert_eq!(picked(&cx), Some(ProviderKind::Anthropic));

        let key = "sk-ant-example-key-0000";
        let field = view.read_with(&cx, |view, cx| view.credentials.read(cx).key.clone());
        type_in(&field, key, &mut cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("provider-test", cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("provider-works").is_some(), "It works");
            window.click(("provider-model", 1_usize), cx);
            window.render_frame(cx);
            window.click("provider-use", cx);
        });
        cx.run_until_parked();
        let sent = |model: Option<&str>| ProviderRequest {
            provider: ProviderKind::Anthropic,
            api_key: Some(key.to_owned()),
            base_url: None,
            model: model.map(str::to_owned),
            fast_model: None,
        };
        assert_eq!(
            backend.calls()[2..],
            [
                Call::TestAgentProvider(sent(None)),
                Call::SaveAgentProvider(sent(Some("example-model-b"))),
            ]
        );
        let left = field.read_with(&cx, |input, _| input.value().to_string());
        assert_eq!(left, "", "the key is gone from the field once saved");
        store.read_with(&cx, |store, _| {
            assert_eq!(store.credentials().saves(), 1);
            assert!(!store.credentials().has_key());
        });
    }

    /// The look for a local Ollama as the picker opens.
    fn ollama_look() -> ProviderRequest {
        ProviderRequest {
            provider: ProviderKind::Ollama,
            api_key: None,
            base_url: None,
            model: None,
            fast_model: None,
        }
    }

    #[gpui_kit::test]
    fn another_service_asks_which_api_it_speaks(cx: &mut TestAppContext) {
        let pane = SettingsPane::Credentials;
        let (backend, store, view, mut cx) = open_settings_view(cx, Some(pane));
        store.update(&mut cx, |store, cx| {
            store.show_in_test_column(Some(Page::SettingsPane(pane)), cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("provider-openai-compatible", cx);
            window.render_frame(cx);
            assert!(
                window
                    .try_find("provider-speaks-openai-compatible")
                    .is_some()
            );
        });
        let picked = store.read_with(&cx, |store, _| store.credentials().picked().cloned());
        assert_eq!(
            picked,
            Some(ProviderKind::OpenAiCompatible),
            "OpenAI-style first"
        );
        cx.update(|window, cx| window.click("provider-speaks-anthropic-compatible", cx));
        cx.run_until_parked();
        let (url, key) = view.read_with(&cx, |view, cx| {
            let pane = view.credentials.read(cx);
            (pane.base_url.clone(), pane.key.clone())
        });
        type_in(&url, "https://example.test", &mut cx);
        type_in(&key, "sk-example-key-0000", &mut cx);
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("provider-test", cx);
        });
        cx.run_until_parked();
        assert_eq!(
            backend.calls().last(),
            Some(&Call::TestAgentProvider(ProviderRequest {
                provider: ProviderKind::AnthropicCompatible,
                api_key: Some("sk-example-key-0000".to_owned()),
                base_url: Some("https://example.test".to_owned()),
                model: None,
                fast_model: None,
            }))
        );
    }

    /// What a server answers with a `DeepSeek` key saved.
    fn deepseek_saved() -> AgentProvider {
        AgentProvider {
            provider: Some(ProviderKind::DeepSeek),
            key_set: true,
            key_hint: Some("0000".to_owned()),
            usable: true,
            ..AgentProvider::NONE
        }
    }

    /// Draws the window in the light appearance, then the dark, finding `ids` in each.
    fn draws_light_and_dark(cx: &mut VisualTestContext, ids: &[&'static str]) {
        for dark in [false, true] {
            cx.update(|window, cx| {
                cx.set_global(Pal::new(&crate::theme::built_in(), dark));
                window.render_frame(cx);
                for id in ids {
                    assert!(window.try_find(*id).is_some(), "{id}, dark: {dark}");
                }
            });
        }
    }

    #[gpui_kit::test]
    fn the_thinks_with_row_shows_what_is_saved(cx: &mut TestAppContext) {
        let (backend, store, mut cx) = open_settings(cx);
        backend.provider.push_status(Ok(deepseek_saved()));
        store.update(&mut cx, |store, cx| {
            store.show_in_test_column(Some(Page::Settings), cx);
        });
        cx.run_until_parked();
        draws_light_and_dark(&mut cx, &["credentials-row"]);
        let line = store.read_with(&cx, |store, _| store.credentials().saved().map(saved_line));
        assert_eq!(line.as_deref(), Some("DeepSeek · key ending 0000"));
    }

    #[gpui_kit::test]
    fn the_thinks_with_pane_opens_on_the_saved_provider(cx: &mut TestAppContext) {
        let pane = SettingsPane::Credentials;
        let (backend, store, _, mut cx) = open_settings_view(cx, Some(pane));
        let mut saved = deepseek_saved();
        saved.in_use = false;
        backend.provider.push_status(Ok(saved));
        store.update(&mut cx, |store, cx| {
            store.show_in_test_column(Some(Page::SettingsPane(pane)), cx);
        });
        cx.run_until_parked();
        draws_light_and_dark(
            &mut cx,
            &[
                "provider-anthropic",
                "provider-openai-compatible",
                "provider-test",
                "provider-forget",
                "provider-not-in-use",
            ],
        );
        cx.update(|window, cx| {
            window.click("provider-forget", cx);
            window.render_frame(cx);
            window.click("provider-forget-confirm", cx);
        });
        cx.run_until_parked();
        assert_eq!(backend.calls().last(), Some(&Call::ForgetAgentProvider));
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
