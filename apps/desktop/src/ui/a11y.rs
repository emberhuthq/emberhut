//! What a screen reader (`VoiceOver` on the Mac, through GPUI's `AccessKit` tree) hears of the
//! app's own controls, and the words it hears.
//!
//! GPUI reports an element only when it has an id and a role, so a clickable `div` is silent
//! until [`Accessible`] gives it one, and decoration (status dots, the avatar, icons beside a
//! label) stays out of the tree by having neither. Markdown views report their text as a
//! document of their own, and gpui-kit's buttons name themselves after their label.
//!
//! The words are plain functions here, so they are tested without a window.

use emberhut_api::tasks::ProjectHealth;
use gpui_kit::accesskit::Live;
use gpui_kit::prelude::*;
use gpui_kit::{Div, ElementId, Role, SharedString, Stateful, StatefulInteractiveElement, div};

/// Roles, names, hints and announcements for an element with an id.
pub(crate) trait Accessible: StatefulInteractiveElement + Sized {
    /// A button the screen reader names `label` (for one that shows an icon, or more than its
    /// name).
    fn button_named(self, label: impl Into<SharedString>) -> Self {
        self.role(Role::Button).aria_label(label)
    }

    /// A row or card read as one element: `label` says all of it, as one line.
    fn read_as(self, role: Role, label: impl Into<SharedString>) -> Self {
        self.role(role).aria_label(label)
    }

    /// What happens on a press, said after the name ("Shows its details").
    fn hint(self, hint: impl Into<SharedString>) -> Self {
        self.aria_description(hint)
    }

    /// A line the screen reader says by itself when it shows or changes (a toast, a status),
    /// without taking the user's place; `label` is what it says.
    fn announced(self, label: impl Into<SharedString>) -> Self {
        self.role(Role::Status).aria_label(label).live()
    }

    /// Changes to its name are said by themselves, without taking the user's place (a file's
    /// download status). It needs a role of its own.
    fn live(self) -> Self {
        self.a11y_synthetic_children(|tree| tree.parent_node().set_live(Live::Polite))
    }
}

impl<E: StatefulInteractiveElement> Accessible for E {}

/// A line a screen reader says by itself as it shows or changes (a status, a problem): a `div`
/// with `id`, announced as `text`. Style it like any `div`.
pub(crate) fn said(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    div().id(id).announced(text)
}

/// The working line of the ember `name`d so: "Juniper is working: Reading a thread", or
/// "… thinking" with no step yet.
#[must_use]
pub(crate) fn working(name: &str, doing: Option<&str>) -> String {
    let doing = doing.map(str::trim).filter(|doing| !doing.is_empty());
    format!("{name} is working: {}", doing.unwrap_or("thinking"))
}

/// "1 needs you", "3 need you".
#[must_use]
pub(crate) fn need_you(count: u32) -> String {
    if count == 1 {
        "1 needs you".to_owned()
    } else {
        format!("{count} need you")
    }
}

/// A room's row as one line: its name, then what waits, what is new and whether the assistant
/// works there ("Main, 2 need you, 3 new, working").
#[must_use]
pub(crate) fn room(name: &str, needs: u32, unread: u32, working: bool) -> String {
    let mut parts = vec![name.to_owned()];
    if needs > 0 {
        parts.push(need_you(needs));
    }
    if unread > 0 {
        parts.push(format!("{unread} new"));
    }
    if working {
        parts.push("working".to_owned());
    }
    parts.join(", ")
}

/// Parts read as one line, the empty ones left out ("Appearance, Blue, light or dark").
#[must_use]
pub(crate) fn line(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A colour preset's name, Blue named the default ("Blue, the default").
#[must_use]
pub(crate) fn preset(name: &str, is_default: bool) -> String {
    if is_default {
        format!("{name}, the default")
    } else {
        name.to_owned()
    }
}

/// A folded section's value: "open" or "folded".
#[must_use]
pub(crate) const fn folded(open: bool) -> &'static str {
    if open { "open" } else { "folded" }
}

/// A project's row: its name, whether it is late or at risk, and its open tasks ("Site, at
/// risk, 4 open").
#[must_use]
pub(crate) fn project(name: &str, health: Option<&ProjectHealth>, open: u32) -> String {
    let health = match health {
        Some(ProjectHealth::Late) => "late",
        Some(ProjectHealth::AtRisk) => "at risk",
        _ => "",
    };
    let open = if open > 0 {
        format!("{open} open")
    } else {
        String::new()
    };
    line(&[name, health, &open])
}

/// An area's row under its project: its name and open tasks ("Design, 2 open").
#[must_use]
pub(crate) fn area(name: &str, open: u32) -> String {
    let open = if open > 0 {
        format!("{open} open")
    } else {
        String::new()
    };
    line(&[name, &open])
}

/// A piece of work on a project's page: its title, its stage and what to do ("Write the
/// brief, Needs you, Pick a colour").
#[must_use]
pub(crate) fn item(title: &str, stage: &str, action: Option<&str>) -> String {
    line(&[title, stage, action.unwrap_or_default()])
}

/// A usage window in words: "5 hours: 42% used, resets 4 pm", or "no number" without data.
#[must_use]
pub(crate) fn usage_window(label: &str, used: Option<f64>, resets: &str) -> String {
    let used = used.map_or_else(
        || "no number".to_owned(),
        |used| format!("{}% used", crate::format::whole(used)),
    );
    format!("{label}: {}", line(&[&used, resets]))
}

/// A day as a screen reader says it: "Tuesday 24 November", and "today" on today.
#[must_use]
pub(crate) fn day(day: jiff::civil::Date, today: bool) -> String {
    let name = day.strftime("%A %-d %B").to_string();
    if today {
        format!("{name}, today")
    } else {
        name
    }
}

/// A day of the month and how much is on it: "Tuesday 24 November, today, 3 things".
#[must_use]
pub(crate) fn day_with(day: jiff::civil::Date, today: bool, count: usize) -> String {
    let count = match count {
        0 => "nothing".to_owned(),
        1 => "1 thing".to_owned(),
        n => format!("{n} things"),
    };
    format!("{}, {count}", self::day(day, today))
}

#[cfg(test)]
mod views;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_working_line_says_what_the_assistant_does() {
        assert_eq!(
            working("Juniper", Some("Reading a thread")),
            "Juniper is working: Reading a thread"
        );
        assert_eq!(working("Pip", None), "Pip is working: thinking");
        assert_eq!(
            working("Your ember", Some("  ")),
            "Your ember is working: thinking"
        );
    }

    #[test]
    fn a_room_reads_as_one_line() {
        assert_eq!(room("Main", 0, 0, false), "Main");
        assert_eq!(room("Main", 1, 0, false), "Main, 1 needs you");
        assert_eq!(room("Ads", 2, 3, true), "Ads, 2 need you, 3 new, working");
    }

    #[test]
    fn a_line_leaves_out_its_empty_parts() {
        assert_eq!(line(&["Appearance", "", " Blue "]), "Appearance, Blue");
        assert_eq!(line(&[]), "");
    }

    #[test]
    fn a_project_and_its_areas_read_as_one_line() {
        assert_eq!(
            project("Site", Some(&ProjectHealth::AtRisk), 4),
            "Site, at risk, 4 open"
        );
        assert_eq!(project("Site", Some(&ProjectHealth::OnTrack), 0), "Site");
        assert_eq!(
            project("Site", Some(&ProjectHealth::Late), 1),
            "Site, late, 1 open"
        );
        assert_eq!(project("Site", None, 2), "Site, 2 open");
        assert_eq!(area("Design", 2), "Design, 2 open");
        assert_eq!(area("Design", 0), "Design");
    }

    #[test]
    fn a_piece_of_work_says_its_stage_and_what_to_do() {
        assert_eq!(
            item("Write the brief", "Needs you", Some("Pick a colour")),
            "Write the brief, Needs you, Pick a colour"
        );
        assert_eq!(item("Ship it", "Done", None), "Ship it, Done");
    }

    #[test]
    fn a_usage_window_says_how_much_is_used_and_when_it_resets() {
        assert_eq!(
            usage_window("5 hours", Some(41.6), "resets 4 pm"),
            "5 hours: 42% used, resets 4 pm"
        );
        assert_eq!(usage_window("week", None, ""), "week: no number");
    }

    #[test]
    fn a_day_is_said_in_full_and_today_says_so() {
        let day = jiff::civil::date(2026, 11, 24);
        assert_eq!(super::day(day, false), "Tuesday 24 November");
        assert_eq!(super::day(day, true), "Tuesday 24 November, today");
        assert_eq!(day_with(day, false, 0), "Tuesday 24 November, nothing");
        assert_eq!(day_with(day, false, 1), "Tuesday 24 November, 1 thing");
        assert_eq!(
            day_with(day, true, 3),
            "Tuesday 24 November, today, 3 things"
        );
    }

    #[test]
    fn presets_and_folds_have_their_words() {
        assert_eq!(preset("Paper", true), "Paper, the default");
        assert_eq!(preset("Green", false), "Green");
        assert_eq!(folded(true), "open");
        assert_eq!(folded(false), "folded");
    }
}
