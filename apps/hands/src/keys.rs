//! Keys and the mouse as input events: a key's name (`Return`, `cmd+s`) as a key code and
//! modifiers, text as one event per character, a click as the reference helper sends it.

use std::time::Duration;

use crate::fail::Fail;

/// The modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one bool per modifier key, as the keyboard has them"
)]
pub struct Modifiers {
    /// Command (⌘).
    pub command: bool,
    /// Shift (⇧).
    pub shift: bool,
    /// Option (⌥).
    pub option: bool,
    /// Control (⌃).
    pub control: bool,
}

/// A key with its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    /// The Mac's virtual key code.
    pub code: u16,
    /// The modifiers held with it.
    pub modifiers: Modifiers,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// The left (primary) button.
    Left,
    /// The right (secondary) button.
    Right,
}

/// One input event, or a pause between two.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A key going down or up.
    Key {
        /// The key and its modifiers.
        chord: Chord,
        /// Down (pressed) or up (let go).
        down: bool,
    },
    /// One character typed as it is (its UTF-16 units), down or up.
    Text {
        /// The character's UTF-16 code units.
        units: Vec<u16>,
        /// Down or up.
        down: bool,
    },
    /// The pointer moved to `x`, `y` (screen points).
    Move {
        /// Points from the main screen's left.
        x: f64,
        /// Points from the main screen's top.
        y: f64,
    },
    /// A mouse button going down or up at `x`, `y`.
    Click {
        /// Which button.
        button: Button,
        /// Down or up.
        down: bool,
        /// Points from the main screen's left.
        x: f64,
        /// Points from the main screen's top.
        y: f64,
        /// 1 for a click, 2 for a double click's second.
        count: i64,
    },
    /// The scroll wheel, `dy` pixels (positive scrolls down the page).
    Scroll {
        /// Pixels down.
        dy: i32,
    },
    /// A pause, so the app keeps up.
    Pause(Duration),
}

impl Input {
    /// Whether the Mac counts this event as input from someone: a pointer move, a button or
    /// key going down, a scroll (what the helper compares to tell the owner's input from its
    /// own).
    #[must_use]
    pub fn counted(&self) -> bool {
        match self {
            Self::Key { down, .. } | Self::Text { down, .. } | Self::Click { down, .. } => *down,
            Self::Move { .. } | Self::Scroll { .. } => true,
            Self::Pause(_) => false,
        }
    }
}

/// The keys by name, with the Mac's virtual key codes (the reference helper's table).
const CODES: &[(&str, u16)] = &[
    ("return", 36),
    ("enter", 76),
    ("tab", 48),
    ("space", 49),
    ("delete", 51),
    ("backspace", 51),
    ("forwarddelete", 117),
    ("escape", 53),
    ("esc", 53),
    ("left", 123),
    ("right", 124),
    ("down", 125),
    ("up", 126),
    ("home", 115),
    ("end", 119),
    ("pageup", 116),
    ("pagedown", 121),
    ("a", 0),
    ("s", 1),
    ("d", 2),
    ("f", 3),
    ("h", 4),
    ("g", 5),
    ("z", 6),
    ("x", 7),
    ("c", 8),
    ("v", 9),
    ("b", 11),
    ("q", 12),
    ("w", 13),
    ("e", 14),
    ("r", 15),
    ("y", 16),
    ("t", 17),
    ("1", 18),
    ("2", 19),
    ("3", 20),
    ("4", 21),
    ("6", 22),
    ("5", 23),
    ("=", 24),
    ("9", 25),
    ("7", 26),
    ("-", 27),
    ("8", 28),
    ("0", 29),
    ("o", 31),
    ("u", 32),
    ("i", 34),
    ("p", 35),
    ("l", 37),
    ("j", 38),
    ("k", 40),
    ("n", 45),
    ("m", 46),
    (",", 43),
    (".", 47),
    ("/", 44),
    ("f1", 122),
    ("f2", 120),
    ("f3", 99),
    ("f4", 118),
    ("f5", 96),
];

/// The most characters one request types.
pub const MAX_TEXT: usize = 4000;

/// The keys that only move around (Tab, Escape, the arrows, Home, End, the pages), with or
/// without Shift: they never put anything into a field.
const NAVIGATION: &[&str] = &[
    "tab", "escape", "esc", "left", "right", "up", "down", "home", "end", "pageup", "pagedown",
];

/// How long to wait after each typed character.
const PER_CHARACTER: Duration = Duration::from_millis(8);
/// How long the pointer rests before a click.
const BEFORE_CLICK: Duration = Duration::from_millis(60);
/// How long between the clicks of a double click.
const BETWEEN_CLICKS: Duration = Duration::from_millis(40);

/// A key's name as a chord: `Return`, `cmd+s`, `shift+tab`, `ctrl+alt+delete`.
///
/// # Errors
///
/// [`Fail`] (400) for a modifier or key it doesn't know.
pub fn chord(name: &str) -> Result<Chord, Fail> {
    let lower = name.to_lowercase();
    let mut parts: Vec<&str> = lower.split('+').map(str::trim).collect();
    let key = parts.pop().unwrap_or_default();
    let mut modifiers = Modifiers::default();
    for part in parts {
        match part {
            "cmd" | "command" | "meta" => modifiers.command = true,
            "shift" => modifiers.shift = true,
            "alt" | "option" | "opt" => modifiers.option = true,
            "ctrl" | "control" => modifiers.control = true,
            other => return Err(Fail::bad(format!("Unknown key {other} in {name}"))),
        }
    }
    let code = CODES
        .iter()
        .find(|(known, _)| *known == key)
        .map(|(_, code)| *code)
        .ok_or_else(|| {
            Fail::bad(format!(
                "Unknown key {name}: a name (Return, Tab, Escape, Up, Delete…) or cmd+s, shift+tab…"
            ))
        })?;
    Ok(Chord { code, modifiers })
}

/// Whether the key named `name` only moves around: Tab, Escape, the arrows, Home, End or the
/// pages, with or without Shift.
#[must_use]
pub fn is_navigation(name: &str) -> bool {
    let lower = name.to_lowercase();
    let mut parts: Vec<&str> = lower.split('+').map(str::trim).collect();
    let key = parts.pop().unwrap_or_default();
    parts.iter().all(|part| *part == "shift") && NAVIGATION.contains(&key)
}

/// `text` cut after each Tab, Return or new line: each of those may move the focus, so it is
/// checked again before the next piece is typed.
#[must_use]
pub fn pieces(text: &str) -> Vec<&str> {
    text.split_inclusive(['\t', '\r', '\n']).collect()
}

/// Refuses text longer than [`MAX_TEXT`] characters.
///
/// # Errors
///
/// [`Fail`] (400) saying the limit.
pub fn check_length(text: &str) -> Result<(), Fail> {
    if text.chars().count() > MAX_TEXT {
        return Err(Fail::bad(format!(
            "That's more than {MAX_TEXT} characters to type at once: send it in parts."
        )));
    }
    Ok(())
}

/// A chord pressed and let go.
#[must_use]
pub fn press(chord: Chord) -> Vec<Input> {
    vec![
        Input::Key { chord, down: true },
        Input::Key { chord, down: false },
    ]
}

/// `text` typed as it is, one character at a time.
#[must_use]
pub fn typing(text: &str) -> Vec<Input> {
    let mut inputs = Vec::with_capacity(text.chars().count() * 3);
    for c in text.chars() {
        let mut buffer = [0_u16; 2];
        let units = c.encode_utf16(&mut buffer).to_vec();
        inputs.push(Input::Text {
            units: units.clone(),
            down: true,
        });
        inputs.push(Input::Text { units, down: false });
        inputs.push(Input::Pause(PER_CHARACTER));
    }
    inputs
}

/// `times` clicks of `button` at `x`, `y`: the pointer moves there first.
#[must_use]
pub fn clicks(button: Button, x: f64, y: f64, times: i64) -> Vec<Input> {
    let mut inputs = vec![Input::Move { x, y }, Input::Pause(BEFORE_CLICK)];
    for count in 1..=times {
        for down in [true, false] {
            inputs.push(Input::Click {
                button,
                down,
                x,
                y,
                count,
            });
        }
        inputs.push(Input::Pause(BETWEEN_CLICKS));
    }
    inputs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_modifiers_read_as_chords() {
        assert_eq!(
            chord("Return").unwrap(),
            Chord {
                code: 36,
                modifiers: Modifiers::default()
            }
        );
        let save = chord("cmd+S").unwrap();
        assert_eq!(save.code, 1);
        assert!(save.modifiers.command && !save.modifiers.shift);
        let back = chord("shift + tab").unwrap();
        assert_eq!(back.code, 48);
        assert!(back.modifiers.shift);
        let all = chord("ctrl+option+cmd+shift+delete").unwrap();
        assert_eq!(
            all.modifiers,
            Modifiers {
                command: true,
                shift: true,
                option: true,
                control: true
            }
        );
    }

    #[test]
    fn unknown_keys_and_modifiers_are_refused() {
        assert_eq!(
            chord("hyper+a").unwrap_err().text,
            "Unknown key hyper in hyper+a"
        );
        assert!(
            chord("cmd+F13")
                .unwrap_err()
                .text
                .starts_with("Unknown key cmd+F13")
        );
        assert!(chord("").is_err());
    }

    #[test]
    fn text_is_typed_a_character_at_a_time() {
        let inputs = typing("a😀");
        assert_eq!(inputs.len(), 6);
        assert_eq!(
            inputs[0],
            Input::Text {
                units: vec![97],
                down: true
            }
        );
        assert_eq!(
            inputs[3],
            Input::Text {
                units: vec![0xD83D, 0xDE00],
                down: true
            }
        );
        assert_eq!(inputs[5], Input::Pause(PER_CHARACTER));
    }

    #[test]
    fn a_double_click_counts_its_clicks() {
        let inputs = clicks(Button::Left, 10.0, 20.0, 2);
        assert_eq!(inputs[0], Input::Move { x: 10.0, y: 20.0 });
        let counts: Vec<i64> = inputs
            .iter()
            .filter_map(|input| match input {
                Input::Click { count, .. } => Some(*count),
                _ => None,
            })
            .collect();
        assert_eq!(counts, [1, 1, 2, 2]);
    }

    #[test]
    fn text_is_cut_where_the_focus_may_move() {
        assert_eq!(pieces("user\tpass\nnext"), ["user\t", "pass\n", "next"]);
        assert_eq!(pieces("plain"), ["plain"]);
        assert_eq!(pieces(""), Vec::<&str>::new());
    }

    #[test]
    fn only_moving_keys_are_navigation() {
        assert!(is_navigation("Tab"));
        assert!(is_navigation("shift+tab"));
        assert!(is_navigation("Down"));
        assert!(!is_navigation("cmd+tab"));
        assert!(!is_navigation("Return"));
        assert!(!is_navigation("a"));
    }

    #[test]
    fn long_text_is_refused() {
        assert!(check_length(&"a".repeat(MAX_TEXT)).is_ok());
        assert!(check_length(&"a".repeat(MAX_TEXT + 1)).is_err());
    }

    #[test]
    fn a_press_is_down_then_up() {
        let c = chord("escape").unwrap();
        assert_eq!(
            press(c),
            [
                Input::Key {
                    chord: c,
                    down: true
                },
                Input::Key {
                    chord: c,
                    down: false
                }
            ]
        );
    }
}
