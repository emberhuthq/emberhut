//! Every setting, described once: its id, title, one line (fixed, or worked out from what the
//! app knows now), its group and where in it, the other words people call it by, whether it is
//! for every day or technical, and where opening it goes. Settings' search, its suggestions and
//! its group pages' anchors all read this one list, so a setting is findable by being
//! described here. The phone keeps the same ids (`apps/fixtures/settings-entries.json` checks
//! both).

use gpui_kit::assets::IconName;

use super::SettingsPane;
use super::now::{Now, computer};
use crate::store::AgentsPlace;

/// A group of Settings: a chip on the front door, and a page of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    /// The agent itself: on duty, what it thinks with, what it knows.
    Ember,
    /// Light or dark, and the colour.
    Appearance,
    /// The devices that reach the agent, pairing another, the machines it works on.
    Devices,
    /// What the agent reads and uses: data sources, Chrome, tools, skills.
    Connections,
    /// Who else can talk to the agent.
    Sharing,
    /// What the agent keeps, and where the user's data goes.
    Privacy,
    /// Updates, the server, diagnostics, where the agent works.
    Advanced,
    /// The emberhut.com account, in builds that have it.
    Account,
}

impl Group {
    /// Every group, in the chips' order.
    pub(crate) const ALL: [Self; 8] = [
        Self::Ember,
        Self::Appearance,
        Self::Devices,
        Self::Connections,
        Self::Sharing,
        Self::Privacy,
        Self::Advanced,
        Self::Account,
    ];

    /// Its name, as its chip and its page's title say it.
    #[must_use]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Ember => "Your ember",
            Self::Appearance => "Appearance",
            Self::Devices => "Devices",
            Self::Connections => "Connections",
            Self::Sharing => "Sharing",
            Self::Privacy => "Privacy",
            Self::Advanced => "Advanced",
            Self::Account => "Account",
        }
    }

    /// Its id, as element ids and the phone's list name it (`ember`, `connections`).
    #[must_use]
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Ember => "ember",
            Self::Appearance => "appearance",
            Self::Devices => "devices",
            Self::Connections => "connections",
            Self::Sharing => "sharing",
            Self::Privacy => "privacy",
            Self::Advanced => "advanced",
            Self::Account => "account",
        }
    }

    /// Its page.
    #[must_use]
    pub(crate) fn pane(self) -> SettingsPane {
        match self {
            Self::Ember => SettingsPane::Ember,
            Self::Appearance => SettingsPane::Appearance,
            Self::Devices => SettingsPane::Devices,
            Self::Connections => SettingsPane::Connections,
            Self::Sharing => SettingsPane::Sharing,
            Self::Privacy => SettingsPane::Privacy,
            Self::Advanced => SettingsPane::Advanced,
            Self::Account => SettingsPane::Account,
        }
    }

    /// The group whose page `pane` is, if it is one.
    #[must_use]
    pub(crate) fn of_pane(pane: SettingsPane) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.pane() == pane)
    }

    /// Whether it shows now: Account only in builds with the account.
    #[must_use]
    pub(crate) fn shown(self, now: &Now) -> bool {
        self != Self::Account || now.has_account
    }

    /// The line at the right of its page's title: how it stands, in a few words.
    #[must_use]
    pub(crate) fn status(self, now: &Now) -> String {
        match self {
            Self::Ember => now.doing_word(),
            Self::Appearance => now.appearance_word(),
            Self::Devices => match now.devices {
                Some(1) => "1 can reach your ember".to_owned(),
                Some(count) => format!("{count} can reach your ember"),
                None => String::new(),
            },
            Self::Connections => {
                let mut parts = Vec::new();
                if now.sources_on > 0 {
                    parts.push(format!("{} on", now.sources_on));
                }
                match now.needs_look() {
                    0 => {}
                    1 => parts.push("1 needs a look".to_owned()),
                    count => parts.push(format!("{count} need a look")),
                }
                parts.join(" · ")
            }
            Self::Sharing => "Only you".to_owned(),
            Self::Privacy => now
                .facts
                .map_or_else(String::new, |count| format!("{count} things known")),
            Self::Advanced => "Nothing here is needed day to day".to_owned(),
            Self::Account => "Optional".to_owned(),
        }
    }

    /// The count on its chip: what needs a look there (Connections only).
    #[must_use]
    pub(crate) fn badge(self, now: &Now) -> Option<usize> {
        (self == Self::Connections)
            .then(|| now.needs_look())
            .filter(|count| *count > 0)
    }
}

/// The agent's state, in the one set of words.
#[must_use]
pub(crate) fn duty_word(on: bool) -> &'static str {
    if on { "On duty" } else { "Off duty" }
}

/// A part of a group page folded under a disclosure: the technical rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Fold {
    /// Your ember's technical details: the models it runs on.
    EmberDetails,
    /// The MCP tool servers.
    Tools,
    /// The skills.
    Skills,
    /// Advanced's diagnostics: sign-in mode, the poll's numbers.
    Diagnostics,
}

impl Fold {
    /// Its id, for its disclosure row (`fold-tools`).
    #[must_use]
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::EmberDetails => "fold-ember-details",
            Self::Tools => "fold-tools",
            Self::Skills => "fold-skills",
            Self::Diagnostics => "fold-diagnostics",
        }
    }
}

/// Where opening a setting goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    /// Its group's page, scrolled to its row (the entry's id is the row's anchor).
    Group,
    /// A page of its own over Settings.
    Pane(SettingsPane),
    /// What your ember knows about you, with Learning from you open.
    Lessons,
    /// The machines.
    Machines,
}

/// Whether a setting is for every day or technical (folded, and ranked after).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Shown on its page as it opens.
    Everyday,
    /// Technical: under a disclosure, or of use to few.
    Technical,
}

/// A setting's one line.
#[derive(Clone, Copy)]
pub(crate) enum Line {
    /// The same words always.
    Fixed(&'static str),
    /// Worked out from what the app knows now.
    Live(fn(&Now) -> String),
}

impl std::fmt::Debug for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(text) => f.debug_tuple("Fixed").field(text).finish(),
            Self::Live(_) => f.write_str("Live"),
        }
    }
}

/// One setting, as search, the suggestions and the group pages know it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    /// Its id: the phone's list uses the same, and a group page anchors its row by it.
    pub(crate) id: &'static str,
    /// Its title.
    pub(crate) title: &'static str,
    /// Its one line.
    pub(crate) line: Line,
    /// Its group.
    pub(crate) group: Group,
    /// Where in its group, after the group's name (`Tools (MCP)`), if not at its top.
    pub(crate) place: Option<&'static str>,
    /// Other words people use for it.
    pub(crate) synonyms: &'static [&'static str],
    /// Every day or technical.
    pub(crate) kind: Kind,
    /// Where opening it goes.
    pub(crate) to: Destination,
    /// The disclosure its row is under on its page, opened when it is opened.
    pub(crate) fold: Option<Fold>,
    /// Its icon.
    pub(crate) icon: IconName,
}

impl Entry {
    /// Its one line now.
    #[must_use]
    pub(crate) fn line(&self, now: &Now) -> String {
        match self.line {
            Line::Fixed(text) => text.to_owned(),
            Line::Live(line) => line(now),
        }
    }

    /// Where it lives, as a result's right side says it: `Connections › Tools (MCP)`.
    #[must_use]
    pub(crate) fn path(&self) -> String {
        match self.place {
            Some(place) => format!("{} › {place}", self.group.name()),
            None => self.group.name().to_owned(),
        }
    }

    /// Whether it shows now (its group does).
    #[must_use]
    pub(crate) fn shown(&self, now: &Now) -> bool {
        self.group.shown(now)
    }
}

impl PartialEq for Entry {
    /// Two entries are the same setting when their ids are.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Entry {}

/// The entry with id `id`.
#[must_use]
pub(crate) fn entry(id: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.id == id)
}

/// An everyday entry on its group page's top.
const fn everyday(
    id: &'static str,
    title: &'static str,
    line: Line,
    group: Group,
    to: Destination,
    icon: IconName,
) -> Entry {
    Entry {
        id,
        title,
        line,
        group,
        place: None,
        synonyms: &[],
        kind: Kind::Everyday,
        to,
        fold: None,
        icon,
    }
}

impl Entry {
    /// The same entry, `place` in its group.
    const fn at(mut self, place: &'static str) -> Self {
        self.place = Some(place);
        self
    }

    /// The same entry, also found by `synonyms`.
    const fn also(mut self, synonyms: &'static [&'static str]) -> Self {
        self.synonyms = synonyms;
        self
    }

    /// The same entry, technical, under `fold` on its page if any.
    const fn technical(mut self, fold: Option<Fold>) -> Self {
        self.kind = Kind::Technical;
        self.fold = fold;
        self
    }
}

/// Every setting, in the order a tie in search keeps.
pub(crate) const ENTRIES: &[Entry] = &[
    // Your ember.
    everyday(
        "on-duty",
        "On duty",
        Line::Live(|now| {
            if now.on_duty {
                format!("On duty · {}", now.doing)
            } else {
                "Off duty · still answers when you message it".to_owned()
            }
        }),
        Group::Ember,
        Destination::Group,
        IconName::Power,
    )
    .also(&[
        "pause",
        "resume",
        "off duty",
        "stop",
        "sleep",
        "wake",
        "status",
        "connection",
    ]),
    everyday(
        "thinks-with",
        "What your ember thinks with",
        Line::Live(|now| {
            now.provider
                .clone()
                .unwrap_or_else(|| "Claude, OpenAI, DeepSeek, OpenRouter, Ollama…".to_owned())
        }),
        Group::Ember,
        Destination::Pane(SettingsPane::Credentials),
        IconName::Brain,
    )
    .at("What it thinks with")
    .also(&[
        "model",
        "provider",
        "api key",
        "key",
        "llm",
        "claude",
        "openai",
        "deepseek",
        "openrouter",
        "ollama",
    ]),
    everyday(
        "usage",
        "Claude usage",
        Line::Fixed("How much of each account's week is used, and which one your ember runs on"),
        Group::Ember,
        Destination::Group,
        IconName::ChartBar,
    )
    .at("What it thinks with")
    .also(&[
        "limits",
        "quota",
        "accounts",
        "auto",
        "pin",
        "week",
        "5-hour",
        "check now",
        "refresh",
    ]),
    everyday(
        "about-you",
        "What your ember knows about you",
        Line::Live(|now| match now.facts {
            Some(count) => format!("{count} things · fix or forget any"),
            None => "Fix or forget any of it".to_owned(),
        }),
        Group::Ember,
        Destination::Pane(SettingsPane::AboutYou),
        IconName::BookOpen,
    )
    .at("What it knows")
    .also(&[
        "memory",
        "forget",
        "facts",
        "remember",
        "about me",
        "fix a fact",
        "edit",
    ]),
    everyday(
        "lessons",
        "Learning from you",
        Line::Live(|now| match now.lessons {
            0 => "Lessons from your corrections: keep or drop each".to_owned(),
            1 => "1 lesson to review".to_owned(),
            count => format!("{count} lessons to review"),
        }),
        Group::Ember,
        Destination::Lessons,
        IconName::GraduationCap,
    )
    .at("What it knows")
    .also(&["trial", "corrections", "keep", "drop", "trying out"]),
    everyday(
        "models",
        "Technical details",
        Line::Live(|now| {
            now.models
                .clone()
                .unwrap_or_else(|| "The model and the side rooms' fast model".to_owned())
        }),
        Group::Ember,
        Destination::Group,
        IconName::Cpu,
    )
    .also(&["fast model", "side room", "session"])
    .technical(Some(Fold::EmberDetails)),
    // Appearance.
    everyday(
        "appearance-mode",
        "Dark appearance",
        Line::Live(|_| format!("Light · Dark · As {} is", computer())),
        Group::Appearance,
        Destination::Pane(SettingsPane::Appearance),
        IconName::Moon,
    )
    .also(&[
        "theme",
        "dark mode",
        "light",
        "light mode",
        "night",
        "system",
        "mode",
    ]),
    everyday(
        "colour",
        "Colour",
        Line::Live(|now| {
            let line = "Ember on paper and others, in light and dark";
            if now.theme.is_empty() {
                line.to_owned()
            } else {
                format!("{line} · now {}", now.theme)
            }
        }),
        Group::Appearance,
        Destination::Pane(SettingsPane::Appearance),
        IconName::Palette,
    )
    .also(&["color", "accent", "preset", "paper", "theme"]),
    everyday(
        "custom-colour",
        "Custom colour",
        Line::Fixed("Your own accent colour, with a preview"),
        Group::Appearance,
        Destination::Pane(SettingsPane::Appearance),
        IconName::Pipette,
    )
    .at("Colour")
    .also(&["hex", "color", "accent", "preview"])
    .technical(None),
    everyday(
        "show-agents",
        "Show agents",
        Line::Live(|now| {
            if now.agents_shown {
                "Your agents' rooms, in the sidebar".to_owned()
            } else {
                "Hidden · open them from here or from Search".to_owned()
            }
        }),
        Group::Appearance,
        Destination::Pane(SettingsPane::Appearance),
        IconName::Users,
    )
    .at("Sidebar")
    .also(&["hide agents", "hide", "agents", "agent rooms", "sidebar"]),
    everyday(
        "agents-place",
        "Where agents sit",
        Line::Live(|now| match now.agents_place {
            AgentsPlace::Below => "Below your rooms and the rest of the sidebar".to_owned(),
            AgentsPlace::Above => "Above your rooms".to_owned(),
        }),
        Group::Appearance,
        Destination::Pane(SettingsPane::Appearance),
        IconName::Layers,
    )
    .at("Sidebar")
    .also(&[
        "agents below rooms",
        "agents above rooms",
        "below",
        "above",
        "move agents",
        "order",
        "sidebar order",
        "agents",
    ]),
    // Devices.
    everyday(
        "devices",
        "Devices",
        Line::Live(|now| match now.devices {
            Some(count) => format!("{count} can reach your ember · sign one out"),
            None => "Each device, when it was last seen, Sign out".to_owned(),
        }),
        Group::Devices,
        Destination::Group,
        IconName::MonitorSmartphone,
    )
    .also(&[
        "phone",
        "computer",
        "sign out",
        "signed in",
        "iphone",
        "android",
        "mac",
    ]),
    everyday(
        "pair",
        "Add a phone or computer",
        Line::Fixed("One code (and QR), works once for 10 minutes"),
        Group::Devices,
        Destination::Group,
        IconName::QrCode,
    )
    .also(&["pair", "qr", "code", "link", "new device", "pair another"]),
    everyday(
        "sign-in-here",
        "Sign this computer in",
        Line::Fixed("Type a code from a signed-in device, or paste its link"),
        Group::Devices,
        Destination::Group,
        IconName::KeyRound,
    )
    .also(&[
        "pairing code",
        "pairing link",
        "pair this computer",
        "log in",
    ]),
    everyday(
        "machines",
        "Machines",
        Line::Fixed("The computers your ember works on"),
        Group::Devices,
        Destination::Machines,
        IconName::Server,
    )
    .at("Computers your ember works on")
    .also(&["computers", "runner", "helpers", "cpu", "gpu", "memory"]),
    // Connections.
    everyday(
        "sources",
        "Data sources",
        Line::Fixed("Gmail, Slack, Chrome… each does only what it says"),
        Group::Connections,
        Destination::Group,
        IconName::Plug,
    )
    .also(&[
        "gmail",
        "slack",
        "mail",
        "email",
        "connect",
        "unlink",
        "reconnect",
        "connections",
    ]),
    everyday(
        "chrome",
        "Chrome profiles",
        Line::Live(|now| {
            if now.chrome.is_empty() {
                "Pair a profile, and your ember reads and clicks in its own tab group".to_owned()
            } else {
                now.chrome.clone()
            }
        }),
        Group::Connections,
        Destination::Pane(SettingsPane::ChromeProfiles),
        IconName::Globe,
    )
    .at("Chrome")
    .also(&[
        "chrome",
        "browser",
        "profile",
        "tab group",
        "pair a profile",
    ]),
    everyday(
        "tools",
        "Tool servers (MCP)",
        Line::Fixed("Switch each on or off; reconnect one that can't connect"),
        Group::Connections,
        Destination::Group,
        IconName::Wrench,
    )
    .at("Tools")
    .also(&[
        "mcp",
        "tools",
        "servers",
        "reconnect",
        "plugins",
        "integrations",
    ])
    .technical(Some(Fold::Tools)),
    everyday(
        "skills",
        "Skills",
        Line::Fixed("What your ember knows how to do, each with a switch"),
        Group::Connections,
        Destination::Group,
        IconName::Sparkles,
    )
    .at("Skills")
    .also(&["abilities", "know how"])
    .technical(Some(Fold::Skills)),
    // Sharing.
    everyday(
        "sharing",
        "Sharing",
        Line::Fixed("Only you can talk to your ember · early preview"),
        Group::Sharing,
        Destination::Group,
        IconName::Users,
    )
    .also(&["share", "invite", "others", "people", "team", "access"]),
    // Privacy.
    everyday(
        "archived-rooms",
        "Archived rooms",
        Line::Live(|now| match now.archived {
            0 => "None · archive a room from the sidebar".to_owned(),
            1 => "1 room · open or bring back".to_owned(),
            count => format!("{count} rooms · open or bring back"),
        }),
        Group::Privacy,
        Destination::Group,
        IconName::Archive,
    )
    .also(&["archive", "unarchive", "old rooms", "restore", "rooms"]),
    everyday(
        "dark-room",
        "Dark room history",
        Line::Fixed("Clear everything said off the record"),
        Group::Privacy,
        Destination::Group,
        IconName::Lock,
    )
    .also(&["off the record", "clear history", "delete", "incognito"]),
    everyday(
        "data-flow",
        "Where your data goes",
        Line::Fixed("What goes to the model you picked, and what stays with you"),
        Group::Privacy,
        Destination::Group,
        IconName::Shield,
    )
    .also(&["data", "privacy", "push", "apple", "sent"]),
    // Advanced.
    everyday(
        "updates",
        "Updates",
        Line::Live(|now| {
            if now.version.is_empty() {
                "The version, when it last checked, Check now".to_owned()
            } else {
                now.version.clone()
            }
        }),
        Group::Advanced,
        Destination::Group,
        IconName::RefreshCw,
    )
    .also(&[
        "version",
        "update",
        "upgrade",
        "restart",
        "check for updates",
        "about",
    ]),
    everyday(
        "server",
        "Your ember's home",
        Line::Live(|now| {
            if now.server.is_empty() {
                "The server's address, phone push".to_owned()
            } else {
                now.server.clone()
            }
        }),
        Group::Advanced,
        Destination::Group,
        IconName::Server,
    )
    .also(&["server", "address", "url", "host", "port"])
    .technical(None),
    everyday(
        "phone-push",
        "Phone alerts",
        Line::Live(|now| {
            if now.push {
                "On · sent through Apple's push service".to_owned()
            } else {
                "Off until the push key is on the server".to_owned()
            }
        }),
        Group::Advanced,
        Destination::Group,
        IconName::Bell,
    )
    .also(&["notifications", "push", "alerts", "apns"]),
    everyday(
        "diagnostics",
        "Diagnostics",
        Line::Fixed("Sign-in mode, how this device is let in, polling"),
        Group::Advanced,
        Destination::Group,
        IconName::Activity,
    )
    .at("Diagnostics")
    .also(&[
        "sign-in mode",
        "metrics",
        "poll",
        "debug",
        "machine id",
        "logs",
    ])
    .technical(Some(Fold::Diagnostics)),
    everyday(
        "workplaces",
        "Where your ember works",
        Line::Fixed("Runners, workers, its browser, coding apps: Check, Repair, Start"),
        Group::Advanced,
        Destination::Group,
        IconName::Hammer,
    )
    .also(&[
        "runner",
        "worker",
        "browser",
        "coding app",
        "environment",
        "repair",
        "start",
    ])
    .technical(None),
    // Account.
    everyday(
        "account",
        "Account",
        Line::Fixed("Your emberhut.com account · optional"),
        Group::Account,
        Destination::Group,
        IconName::CircleUser,
    )
    .also(&["sign in", "emberhut.com", "relay", "login", "log in"]),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Every row of the placement table: where the setting was, its entry, and its new group.
    const PLACEMENT: &[(&str, &str, Group)] = &[
        (
            "Sidebar menu › On duty · paused banner",
            "on-duty",
            Group::Ember,
        ),
        (
            "Claude usage › runs on (Auto / pinned)",
            "usage",
            Group::Ember,
        ),
        (
            "Claude usage › account rows (5h, Week)",
            "usage",
            Group::Ember,
        ),
        ("Claude usage › Check now", "usage", Group::Ember),
        ("Provider picker", "thinks-with", Group::Ember),
        ("About you", "about-you", Group::Ember),
        ("About you › trying out + Drop", "lessons", Group::Ember),
        ("Fix a fact", "about-you", Group::Ember),
        ("Skills", "skills", Group::Connections),
        ("Data sources rows", "sources", Group::Connections),
        ("Chrome profiles page", "chrome", Group::Connections),
        ("MCP servers", "tools", Group::Connections),
        (
            "Appearance › System / Light / Dark",
            "appearance-mode",
            Group::Appearance,
        ),
        ("Appearance › Colour presets", "colour", Group::Appearance),
        (
            "Appearance › Custom colour, Preview",
            "custom-colour",
            Group::Appearance,
        ),
        (
            "Sidebar › Agents section shown",
            "show-agents",
            Group::Appearance,
        ),
        (
            "Sidebar › Agents above or below the rooms",
            "agents-place",
            Group::Appearance,
        ),
        ("Sign in › pairing form", "sign-in-here", Group::Devices),
        (
            "Devices › device rows + Sign out",
            "devices",
            Group::Devices,
        ),
        ("Devices › Pair another device", "pair", Group::Devices),
        ("Devices › Sign-in mode", "diagnostics", Group::Advanced),
        ("This phone › Machine id", "diagnostics", Group::Advanced),
        (
            "This phone › connection status row",
            "on-duty",
            Group::Ember,
        ),
        ("Notifications row", "phone-push", Group::Advanced),
        (
            "Where Emberhut works › Machines",
            "machines",
            Group::Devices,
        ),
        (
            "Where Emberhut works › runners, workers",
            "workplaces",
            Group::Advanced,
        ),
        ("Account", "account", Group::Account),
        (
            "App › Version, Last checked, Check, Restart",
            "updates",
            Group::Advanced,
        ),
        (
            "Server › Address, Phone push, Sign-in, metrics",
            "server",
            Group::Advanced,
        ),
        ("Rooms › Archived rooms", "archived-rooms", Group::Privacy),
        ("Dark room › Clear history", "dark-room", Group::Privacy),
        ("Sharing your ember", "sharing", Group::Sharing),
    ];

    #[test]
    fn every_placed_setting_has_an_entry_with_a_location() {
        for (today, id, group) in PLACEMENT {
            let entry = entry(id).unwrap_or_else(|| panic!("{today}: no entry {id}"));
            assert_eq!(entry.group, *group, "{today}");
            assert!(
                entry.path().starts_with(group.name()),
                "{today}: {}",
                entry.path()
            );
            assert!(!entry.title.is_empty() && !entry.line(&Now::default()).is_empty());
        }
    }

    #[test]
    fn ids_are_unique_and_every_group_has_a_setting() {
        let ids: HashSet<&str> = ENTRIES.iter().map(|entry| entry.id).collect();
        assert_eq!(ids.len(), ENTRIES.len());
        for group in Group::ALL {
            assert!(
                ENTRIES.iter().any(|entry| entry.group == group),
                "{group:?}"
            );
            assert_eq!(Group::of_pane(group.pane()), Some(group));
        }
    }

    #[test]
    fn a_folded_setting_is_a_technical_one_on_its_group_page() {
        for entry in ENTRIES.iter().filter(|entry| entry.fold.is_some()) {
            assert_eq!(entry.kind, Kind::Technical, "{}", entry.id);
            assert_eq!(entry.to, Destination::Group, "{}", entry.id);
        }
    }

    #[test]
    fn the_ids_are_the_phones() {
        let shared: serde_json::Value =
            serde_json::from_str(include_str!("../../../../fixtures/settings-entries.json"))
                .unwrap();
        let theirs: Vec<&str> = shared["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        let ours: Vec<&str> = ENTRIES.iter().map(|entry| entry.id).collect();
        assert_eq!(ours, theirs, "the desktop's and the phone's settings ids");
        let groups: Vec<&str> = shared["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group.as_str().unwrap())
            .collect();
        let names: Vec<&str> = Group::ALL.iter().map(|group| group.name()).collect();
        assert_eq!(names, groups);
    }

    #[test]
    fn the_agent_is_your_ember_in_every_entry() {
        for entry in ENTRIES {
            let line = entry.line(&Now::default());
            for text in [entry.title, line.as_str()] {
                assert!(!text.contains("Emberhut"), "{}: {text}", entry.id);
            }
        }
    }
}
