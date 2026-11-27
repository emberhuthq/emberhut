//! Where a report's navigations go: the page stays put, a link goes to the browser, anything
//! else goes nowhere.
//!
//! The report is loaded from a string with no base address, so the page is `about:blank`. Wry
//! hands the handlers the URL alone, not whether the user clicked a link (which `WebKit`'s own
//! navigation delegate is told), so the two ways out of the page are told apart by how they arrive:
//!
//! - **A new window** (`window.open`, `target="_blank"`, and every link the user clicks, which
//!   the page's guard script turns into `window.open`, see [`super::guard`]): `WebKit` lets a
//!   script open one only while it handles the user's click or key, because the web view's
//!   configuration turns `javaScriptCanOpenWindowsAutomatically` off. So each is the user's
//!   doing: an http, https or mailto one goes to the browser (or mail app), at most one a
//!   second.
//! - **A navigation of the page itself** (a script setting `location`, a meta refresh, a link
//!   the guard didn't see): nothing says the user asked for it. It goes nowhere while the page
//!   loads, and once the page has finished loading at most [`UNPROVEN_LINKS`] of them reach the
//!   browser, one a second, for as long as the page is shown.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use smol::channel::Sender;

/// The page's own address: a page loaded from a string has none.
const PAGE: &str = "about:blank";

/// The least time between two links sent to the browser, so one click opens one page.
pub(super) const BROWSER_GAP: Duration = Duration::from_secs(1);

/// How many navigations of the page itself may reach the browser (see the module's docs).
pub(super) const UNPROVEN_LINKS: usize = 3;

/// What becomes of a navigation the page asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// The web view goes ahead: the page's first load, or a jump within it.
    Stay,
    /// The web view stays put and the URL opens in the default browser.
    Browser,
    /// The web view stays put and nothing opens.
    Refuse(Refusal),
}

/// Why a navigation goes nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// Not an http, https or mailto URL.
    NotAWebLink,
    /// The page navigated itself before it had finished loading.
    Loading,
    /// Less than [`BROWSER_GAP`] after the last link sent to the browser.
    TooSoon,
    /// The page has used up its [`UNPROVEN_LINKS`]; `first` the first time it is told so.
    OverBudget {
        /// Whether this is the first navigation refused for it.
        first: bool,
    },
}

/// The navigation policy of one report's web view.
#[derive(Debug, Default)]
pub(super) struct NavigationPolicy {
    /// Whether the page's own load has been let through.
    started: bool,
    /// Whether the page has finished loading.
    loaded: bool,
    /// When a link last went to the browser.
    last_browser: Option<Instant>,
    /// How many navigations of the page itself went to the browser.
    unproven: usize,
    /// Whether a navigation over the budget was refused already.
    over_budget: bool,
}

impl NavigationPolicy {
    /// The page has finished loading.
    pub(super) fn loaded(&mut self) {
        self.loaded = true;
    }

    /// Whether the page has finished loading.
    pub(super) fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// The web view is about to load `url` in the page (at `now`).
    pub(super) fn navigation(&mut self, url: &str, now: Instant) -> Verdict {
        if is_jump_within_page(url) {
            return Verdict::Stay;
        }
        if url == PAGE && !self.started {
            self.started = true;
            return Verdict::Stay;
        }
        if !is_link(url, &["http", "https", "mailto"]) {
            return Verdict::Refuse(Refusal::NotAWebLink);
        }
        if !self.loaded {
            return Verdict::Refuse(Refusal::Loading);
        }
        if self.unproven >= UNPROVEN_LINKS {
            let first = !self.over_budget;
            self.over_budget = true;
            return Verdict::Refuse(Refusal::OverBudget { first });
        }
        let verdict = self.pass_to_browser(now);
        if verdict == Verdict::Browser {
            self.unproven += 1;
        }
        verdict
    }

    /// The page asks for a new window on `url`, which `WebKit` allows only during the user's
    /// click or key: never a window of the app's.
    pub(super) fn new_window(&mut self, url: &str, now: Instant) -> Verdict {
        if is_link(url, &["http", "https", "mailto"]) {
            self.pass_to_browser(now)
        } else {
            Verdict::Refuse(Refusal::NotAWebLink)
        }
    }

    fn pass_to_browser(&mut self, now: Instant) -> Verdict {
        if self
            .last_browser
            .is_some_and(|last| now.duration_since(last) < BROWSER_GAP)
        {
            return Verdict::Refuse(Refusal::TooSoon);
        }
        self.last_browser = Some(now);
        Verdict::Browser
    }
}

/// Puts the navigation to `url` (at `now`) to `policy` with `ask`, and says whether the web view
/// may go ahead. A link for the browser goes to `links`, and a refusal worth knowing about is
/// logged. This is the glue between `WebKit`'s handlers and the policy: it never touches the app.
pub(super) fn decide(
    policy: &Mutex<NavigationPolicy>,
    links: &Sender<String>,
    url: String,
    now: Instant,
    ask: fn(&mut NavigationPolicy, &str, Instant) -> Verdict,
) -> bool {
    let verdict = ask(
        &mut policy.lock().unwrap_or_else(PoisonError::into_inner),
        &url,
        now,
    );
    match verdict {
        Verdict::Stay => return true,
        Verdict::Browser => {
            if let Err(error) = links.try_send(url) {
                log::warn!("a report's link didn't reach the browser: {error}");
            }
        }
        Verdict::Refuse(Refusal::NotAWebLink) => {
            log::debug!("a report's navigation was refused: {url}");
        }
        Verdict::Refuse(Refusal::Loading) => {
            log::info!("a report's link was refused, the page is still loading: {url}");
        }
        Verdict::Refuse(Refusal::TooSoon) => {
            log::info!("a report's link was refused, a second after the last: {url}");
        }
        Verdict::Refuse(Refusal::OverBudget { first }) => {
            if first {
                log::warn!(
                    "a report navigated itself to the browser {UNPROVEN_LINKS} times: no more \
                     of that until it is opened again ({url} refused)"
                );
            } else {
                log::debug!("a report's navigation was refused, over its budget: {url}");
            }
        }
    }
    false
}

/// `about:blank#…`: a jump to an anchor of the page itself.
fn is_jump_within_page(url: &str) -> bool {
    url.strip_prefix(PAGE)
        .is_some_and(|rest| rest.starts_with('#'))
}

/// Whether `url`'s scheme is one of `schemes` (lowercase), in any case.
fn is_link(url: &str, schemes: &[&str]) -> bool {
    scheme(url).is_some_and(|scheme| {
        schemes
            .iter()
            .any(|wanted| scheme.eq_ignore_ascii_case(wanted))
    })
}

/// The URL's scheme (RFC 3986: a letter, then letters, digits, `+`, `-` or `.`, before `:`).
fn scheme(url: &str) -> Option<&str> {
    let (scheme, _) = url.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    let valid = first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::{BROWSER_GAP, NavigationPolicy, Refusal, UNPROVEN_LINKS, Verdict, decide};

    fn loaded() -> NavigationPolicy {
        let mut policy = NavigationPolicy::default();
        assert_eq!(
            policy.navigation("about:blank", Instant::now()),
            Verdict::Stay
        );
        policy.loaded();
        assert!(policy.is_loaded());
        policy
    }

    #[test]
    fn the_first_load_and_jumps_within_the_page_stay() {
        let now = Instant::now();
        let mut policy = NavigationPolicy::default();
        assert!(!policy.is_loaded());
        assert_eq!(policy.navigation("about:blank", now), Verdict::Stay);
        assert_eq!(policy.navigation("about:blank#top", now), Verdict::Stay);
        policy.loaded();
        assert_eq!(policy.navigation("about:blank#sales", now), Verdict::Stay);
    }

    #[test]
    fn the_page_loads_once_only() {
        let now = Instant::now();
        let mut policy = NavigationPolicy::default();
        assert_eq!(policy.navigation("about:blank", now), Verdict::Stay);
        // A script blanking its page while it loads, and after.
        assert_eq!(
            policy.navigation("about:blank", now),
            Verdict::Refuse(Refusal::NotAWebLink)
        );
        policy.loaded();
        assert_eq!(
            policy.navigation("about:blank", now),
            Verdict::Refuse(Refusal::NotAWebLink)
        );
    }

    #[test]
    fn a_new_window_goes_to_the_browser_loaded_or_not() {
        // WebKit opens one only during the user's click, so it needn't wait for the load: a
        // click while a slow picture loads still works.
        let mut policy = NavigationPolicy::default();
        let mut at = Instant::now();
        assert_eq!(
            policy.new_window("https://emberhut.com/docs", at),
            Verdict::Browser
        );
        policy.loaded();
        for _ in 0..10 {
            at += BROWSER_GAP;
            assert_eq!(
                policy.new_window("HTTP://example.com/", at),
                Verdict::Browser,
                "no budget for the user's clicks"
            );
        }
    }

    #[test]
    fn a_mailto_click_goes_to_the_mail_app_like_a_web_link() {
        // Not through the page's own budget: the user's mailto clicks always go.
        let mut policy = NavigationPolicy::default();
        let mut at = Instant::now();
        for _ in 0..=UNPROVEN_LINKS {
            at += BROWSER_GAP;
            assert_eq!(
                policy.new_window("MAILTO:someone@example.com", at),
                Verdict::Browser
            );
        }
    }

    #[test]
    fn a_new_window_is_never_anything_but_a_web_or_mail_link() {
        let mut policy = loaded();
        let mut at = Instant::now();
        for url in [
            "about:blank#top",
            "file:///etc/hosts",
            "javascript:alert(1)",
        ] {
            at += BROWSER_GAP;
            assert_eq!(
                policy.new_window(url, at),
                Verdict::Refuse(Refusal::NotAWebLink),
                "{url}"
            );
        }
    }

    #[test]
    fn the_page_navigating_itself_waits_for_the_load() {
        let mut policy = NavigationPolicy::default();
        let now = Instant::now();
        assert_eq!(policy.navigation("about:blank", now), Verdict::Stay);
        assert_eq!(
            policy.navigation("https://example.com/", now),
            Verdict::Refuse(Refusal::Loading)
        );
        policy.loaded();
        assert_eq!(
            policy.navigation("mailto:someone@example.com", now),
            Verdict::Browser
        );
    }

    #[test]
    fn the_page_navigating_itself_reaches_the_browser_a_few_times_only() {
        let mut policy = loaded();
        let mut at = Instant::now();
        for _ in 0..UNPROVEN_LINKS {
            at += BROWSER_GAP;
            assert_eq!(
                policy.navigation("https://example.com/", at),
                Verdict::Browser
            );
        }
        at += BROWSER_GAP;
        assert_eq!(
            policy.navigation("https://example.com/", at),
            Verdict::Refuse(Refusal::OverBudget { first: true })
        );
        at += Duration::from_secs(3600);
        assert_eq!(
            policy.navigation("https://example.com/", at),
            Verdict::Refuse(Refusal::OverBudget { first: false }),
            "not even an hour later"
        );
        // The user's clicks still go.
        assert_eq!(
            policy.new_window("https://example.com/", at),
            Verdict::Browser
        );
    }

    #[test]
    fn no_other_scheme_is_followed_or_opened() {
        let mut policy = loaded();
        let mut at = Instant::now();
        for url in [
            "file:///etc/hosts",
            "data:text/html,<p>x</p>",
            "javascript:alert(1)",
            "blob:null/1234",
            "about:srcdoc",
            "emberhut://open",
            "ftp://example.com/",
            "",
            "not a url",
            "1http://example.com/",
        ] {
            at += BROWSER_GAP;
            let refused = Verdict::Refuse(Refusal::NotAWebLink);
            assert_eq!(policy.navigation(url, at), refused, "{url}");
            assert_eq!(policy.new_window(url, at), refused, "{url}");
        }
    }

    #[test]
    fn at_most_one_link_a_second_reaches_the_browser() {
        let mut policy = loaded();
        let start = Instant::now();
        let url = "https://example.com/";
        assert_eq!(policy.new_window(url, start), Verdict::Browser);
        let soon = start + Duration::from_millis(300);
        let too_soon = Verdict::Refuse(Refusal::TooSoon);
        assert_eq!(policy.new_window(url, soon), too_soon);
        assert_eq!(policy.navigation(url, soon), too_soon);
        assert_eq!(
            policy.new_window(url, start + BROWSER_GAP),
            Verdict::Browser
        );
    }

    #[test]
    fn decide_lets_the_page_load_and_hands_links_to_the_browser_task() {
        let policy = Mutex::new(NavigationPolicy::default());
        let (links, to_open) = smol::channel::unbounded();
        let now = Instant::now();
        let navigate = NavigationPolicy::navigation;
        assert!(decide(&policy, &links, "about:blank".into(), now, navigate));
        assert!(!decide(
            &policy,
            &links,
            "https://example.com/early".into(),
            now,
            navigate
        ));
        policy.lock().unwrap().loaded();
        let url = "https://example.com/clicked".to_owned();
        assert!(!decide(
            &policy,
            &links,
            url.clone(),
            now,
            NavigationPolicy::new_window
        ));
        assert!(!decide(
            &policy,
            &links,
            "file:///etc/hosts".into(),
            now + BROWSER_GAP,
            navigate
        ));
        assert_eq!(
            to_open.try_recv().ok(),
            Some(url),
            "only the click went out"
        );
        assert!(to_open.try_recv().is_err());
    }

    #[test]
    fn decide_survives_the_browser_task_being_gone() {
        let policy = Mutex::new(NavigationPolicy::default());
        let (links, to_open) = smol::channel::unbounded();
        drop(to_open);
        assert!(!decide(
            &policy,
            &links,
            "https://example.com/".into(),
            Instant::now(),
            NavigationPolicy::new_window
        ));
    }
}
