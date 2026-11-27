//! What the browser's window and the rows about it say: the site on screen, who drives, the
//! assistant's ask, and whether a task's card shows the page live. No GPUI here.

use emberhut_api::browser::{BrowserControl, BrowserInfo};
use emberhut_api::ids::BrowserDeviceId;
use emberhut_api::work::WorkRow;

use crate::brand::APP_NAME;
use crate::wire::{ASSISTANT_DRIVER, TASK_KEY_PREFIX};

/// The site of the tab on screen for the toolbar: its host without `www.` for a web page,
/// `New tab` for a blank one, else its title (the browser's own pages).
#[must_use]
pub(crate) fn site(info: &BrowserInfo) -> String {
    let Some(tab) = info.shown_tab() else {
        if info.url.is_empty() {
            return "New tab".to_owned();
        }
        return host(&info.url).unwrap_or(&info.url).to_owned();
    };
    if tab.url.is_empty() || tab.url == "about:blank" {
        return "New tab".to_owned();
    }
    if let Some(host) = host(&tab.url) {
        return host.strip_prefix("www.").unwrap_or(host).to_owned();
    }
    if tab.title.is_empty() {
        tab.url.clone()
    } else {
        tab.title.clone()
    }
}

/// Whether the page on screen came over https (the toolbar's lock).
#[must_use]
pub(crate) fn is_secure(info: &BrowserInfo) -> bool {
    info.shown_tab()
        .map_or(info.url.as_str(), |tab| tab.url.as_str())
        .starts_with("https://")
}

/// The whole address of the page on screen, as editing shows it (nothing for a blank tab).
#[must_use]
pub(crate) fn address(info: &BrowserInfo) -> String {
    let url = info
        .shown_tab()
        .map_or(info.url.as_str(), |tab| tab.url.as_str());
    if url == "about:blank" {
        String::new()
    } else {
        url.to_owned()
    }
}

/// The host of an `http` or `https` address (no user, no port).
fn host(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if authority.starts_with('[') {
        authority.split_inclusive(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then_some(host)
}

/// How the pill over the page is tinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    /// The person has it (here or on another device).
    Person,
    /// An agent drives.
    Agent,
    /// Nobody drives.
    Idle,
}

/// Who drives, as the pill over the page says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pill {
    /// Its words.
    pub(crate) text: String,
    /// Its dot's tint.
    pub(crate) tone: Tone,
}

/// The pill for `info` seen from `device`.
#[must_use]
pub(crate) fn pill(info: &BrowserInfo, device: &BrowserDeviceId) -> Pill {
    let tone = if info.control == BrowserControl::Person {
        Tone::Person
    } else if info.driver.is_some() {
        Tone::Agent
    } else {
        Tone::Idle
    };
    let text = if info.is_mine(device) {
        "You're driving".to_owned()
    } else if info.is_unclaimed() {
        format!("{APP_NAME} asked you to take over")
    } else if info.control == BrowserControl::Person {
        "Driving on your other device".to_owned()
    } else if let Some(driver) = &info.driver {
        let task = info
            .task
            .map_or_else(String::new, |task| format!(" · {TASK_KEY_PREFIX}{task}"));
        format!("{} is driving{task}", driver_name(driver))
    } else {
        "Watching".to_owned()
    };
    Pill { text, tone }
}

/// A driver as the app names it: the assistant (and its workers) by the app's name.
fn driver_name(driver: &str) -> String {
    match after_assistant(driver) {
        Some(rest) if is_assistants(driver) => format!("{APP_NAME}{rest}"),
        _ => driver.to_owned(),
    }
}

/// Whether the assistant or one of its workers drives (`<name>` or `<name>'s worker on …`),
/// rather than the person.
fn is_assistants(driver: &str) -> bool {
    after_assistant(driver).is_some_and(|rest| rest.is_empty() || rest.starts_with("'s"))
}

/// What follows the assistant's name at the start of `driver`, its case aside (a server
/// may write the name with a capital).
fn after_assistant(driver: &str) -> Option<&str> {
    let (name, rest) = driver.split_at_checked(ASSISTANT_DRIVER.len())?;
    name.eq_ignore_ascii_case(ASSISTANT_DRIVER).then_some(rest)
}

/// What the sidebar's Browser row says beside it while someone drives: the app's name for
/// the assistant and its workers, `you` for the person.
#[must_use]
pub(crate) fn driver_note(info: &BrowserInfo) -> Option<&'static str> {
    let driver = info.driver.as_deref()?;
    Some(if is_assistants(driver) {
        APP_NAME
    } else {
        "you"
    })
}

/// The assistant's ask under the page, while this device holds the browser it handed over.
#[must_use]
pub(crate) fn ask(info: &BrowserInfo, device: &BrowserDeviceId) -> Option<String> {
    let why = info.handoff.as_deref().filter(|_| info.is_mine(device))?;
    Some(format!(
        "{APP_NAME} asked you: {why}. Give back when you're done."
    ))
}

/// What shows where the page's picture would be, seen from `device` with none here: the
/// server sends no picture while the person drives (here, on another device, or before any
/// device has taken it over), so then it says that rather than wait for one.
#[must_use]
pub(crate) fn no_picture(info: &BrowserInfo, device: &BrowserDeviceId) -> String {
    if info.control != BrowserControl::Person || info.is_mine(device) {
        "Waiting for a picture of the page…".to_owned()
    } else if info.is_unclaimed() {
        format!("{APP_NAME} handed the browser over: take over to drive it here.")
    } else {
        "You're driving on your other device, so no picture of the page comes here. Take over \
         to drive it here."
            .to_owned()
    }
}

/// The note under the page when it wants a file: what is being sent, else that it asks.
#[must_use]
pub(crate) fn file_note(sending: Option<&str>) -> String {
    sending.map_or_else(
        || "The page wants a file".to_owned(),
        |name| format!("Sending {name}…"),
    )
}

/// Whether `row`'s task is the one in the browser now, so its card shows the page live.
#[must_use]
pub(crate) fn is_browsing(row: &WorkRow, info: Option<&BrowserInfo>) -> bool {
    info.is_some_and(|info| row.task.is_some() && info.task == row.task && info.driver.is_some())
}

#[cfg(test)]
mod tests {
    use emberhut_api::browser::BrowserTab;
    use emberhut_api::ids::{BrowserTabId, TaskId};
    use serde_json::json;

    use super::*;

    fn tab(url: &str, title: &str) -> BrowserTab {
        BrowserTab {
            id: BrowserTabId::new(1),
            url: url.to_owned(),
            title: title.to_owned(),
            active: true,
        }
    }

    fn on(tab: BrowserTab) -> BrowserInfo {
        BrowserInfo {
            tabs: vec![tab],
            ..BrowserInfo::default()
        }
    }

    #[test]
    fn the_site_is_the_host_without_www() {
        let info = on(tab("https://www.example.com/a?b#c", "Example"));
        assert_eq!(site(&info), "example.com");
        assert!(is_secure(&info));
        assert_eq!(address(&info), "https://www.example.com/a?b#c");
        let info = on(tab("http://user@example.net:8080/", ""));
        assert_eq!(site(&info), "example.net");
        assert!(!is_secure(&info));
    }

    #[test]
    fn a_blank_tab_is_a_new_tab_and_the_browsers_own_pages_their_title() {
        let blank = on(tab("about:blank", ""));
        assert_eq!(site(&blank), "New tab");
        assert_eq!(address(&blank), "");
        assert_eq!(site(&on(tab("chrome://settings/", "Settings"))), "Settings");
        assert_eq!(site(&on(tab("chrome://version/", ""))), "chrome://version/");
    }

    #[test]
    fn without_tabs_the_state_says_the_page() {
        let mut info = BrowserInfo::default();
        assert_eq!(site(&info), "New tab");
        info.url = "https://www.example.com/x".to_owned();
        assert_eq!(
            site(&info),
            "www.example.com",
            "as the reference: the host whole"
        );
        assert!(is_secure(&info));
    }

    #[test]
    fn the_pill_says_who_drives_from_this_device() {
        let here = BrowserDeviceId::new("here");
        let mut info = BrowserInfo::default();
        assert_eq!(pill(&info, &here).text, "Watching");
        assert_eq!(pill(&info, &here).tone, Tone::Idle);
        info.driver = Some(ASSISTANT_DRIVER.to_owned());
        info.task = Some(TaskId::new(12));
        assert_eq!(
            pill(&info, &here),
            Pill {
                text: format!("{APP_NAME} is driving · {TASK_KEY_PREFIX}12"),
                tone: Tone::Agent
            }
        );
        info.driver = Some(format!(
            "{ASSISTANT_DRIVER}'s worker on {TASK_KEY_PREFIX}13"
        ));
        info.task = None;
        assert_eq!(
            pill(&info, &here).text,
            format!("{APP_NAME}'s worker on {TASK_KEY_PREFIX}13 is driving")
        );
        info.control = BrowserControl::Person;
        info.driver = Some("You".to_owned());
        info.device = Some(BrowserDeviceId::new(""));
        assert_eq!(
            pill(&info, &here).text,
            format!("{APP_NAME} asked you to take over")
        );
        assert_eq!(pill(&info, &here).tone, Tone::Person);
        info.device = Some(BrowserDeviceId::new("other"));
        assert_eq!(pill(&info, &here).text, "Driving on your other device");
        info.device = Some(here.clone());
        assert_eq!(pill(&info, &here).text, "You're driving");
    }

    #[test]
    fn with_no_picture_the_page_says_why_while_the_person_drives_elsewhere() {
        let here = BrowserDeviceId::new("here");
        let mut info = BrowserInfo::default();
        let waiting = "Waiting for a picture of the page…";
        assert_eq!(no_picture(&info, &here), waiting);
        info.control = BrowserControl::Person;
        info.device = Some(BrowserDeviceId::new("other"));
        assert!(no_picture(&info, &here).starts_with("You're driving on your other device"));
        info.device = Some(BrowserDeviceId::new(""));
        assert!(no_picture(&info, &here).contains("take over to drive it here"));
        info.device = Some(here.clone());
        assert_eq!(no_picture(&info, &here), waiting);
    }

    #[test]
    fn the_sidebar_names_the_assistant_or_the_person() {
        let mut info = BrowserInfo::default();
        assert_eq!(driver_note(&info), None);
        info.driver = Some(ASSISTANT_DRIVER.to_owned());
        assert_eq!(driver_note(&info), Some(APP_NAME));
        // A server may write the name with a capital.
        let mut capital = ASSISTANT_DRIVER.to_owned();
        capital[..1].make_ascii_uppercase();
        info.driver = Some(capital);
        assert_eq!(driver_note(&info), Some(APP_NAME));
        info.driver = Some(format!("{ASSISTANT_DRIVER}'s worker on {TASK_KEY_PREFIX}2"));
        assert_eq!(driver_note(&info), Some(APP_NAME));
        info.driver = Some("You".to_owned());
        assert_eq!(driver_note(&info), Some("you"));
        info.driver = Some(format!("{ASSISTANT_DRIVER}ville")); // not the assistant
        assert_eq!(driver_note(&info), Some("you"));
    }

    #[test]
    fn the_ask_shows_only_on_the_device_that_holds_it() {
        let here = BrowserDeviceId::new("here");
        let mut info = BrowserInfo {
            control: BrowserControl::Person,
            handoff: Some("Sign in to the example site".to_owned()),
            device: Some(BrowserDeviceId::new("other")),
            ..BrowserInfo::default()
        };
        assert_eq!(ask(&info, &here), None);
        info.device = Some(here.clone());
        assert_eq!(
            ask(&info, &here).unwrap(),
            format!(
                "{APP_NAME} asked you: Sign in to the example site. Give back when you're done."
            )
        );
        assert_eq!(file_note(None), "The page wants a file");
        assert_eq!(file_note(Some("scan.pdf")), "Sending scan.pdf…");
    }

    #[test]
    fn a_card_is_live_while_its_task_is_being_browsed() {
        let row: WorkRow = serde_json::from_value(json!({
            "id": "task:12", "kind": "task", "task": 12, "title": "Example", "who": "example", "state": "running",
            "at": "2026-11-24T22:00:00.000Z"
        }))
        .unwrap();
        let mut info = BrowserInfo {
            task: Some(TaskId::new(12)),
            ..BrowserInfo::default()
        };
        assert!(!is_browsing(&row, Some(&info)), "nobody drives");
        info.driver = Some(ASSISTANT_DRIVER.to_owned());
        assert!(is_browsing(&row, Some(&info)));
        info.task = Some(TaskId::new(13));
        assert!(!is_browsing(&row, Some(&info)));
        assert!(!is_browsing(&row, None));
    }
}
