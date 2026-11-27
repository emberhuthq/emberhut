use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};

use super::session::HOLD;
use super::*;
use crate::desktop::{Focus, InstalledApp};
use crate::keys::{Button, Input};
use crate::testing::{Rig, node, rig};

fn body(value: Value) -> Reply {
    object(value)
}

fn text(reply: &Reply) -> &str {
    reply.get("text").and_then(Value::as_str).unwrap()
}

/// Calculator (pid 10) with buttons, a field, a password field and a menu bar.
struct Calc {
    seven: usize,
    field: usize,
    password: usize,
    file_menu: usize,
    export: usize,
}

fn calculator(rig: &Rig) -> Calc {
    let d = &rig.desktop;
    let root = d.run(10, "Calculator", "com.apple.calculator");
    let mut seven = node("AXButton", "7");
    seven.actions = vec!["AXPress".to_owned()];
    let seven = d.add(seven);
    let mut field = node("AXTextField", "");
    field.description = Some("Display".to_owned());
    field.value = Some("42".to_owned());
    field.settable = true;
    let field = d.add(field);
    let mut password = node("AXTextField", "");
    password.subrole = "AXSecureTextField".to_owned();
    password.description = Some("Code".to_owned());
    password.value = Some("hunter2".to_owned());
    password.settable = true;
    let password = d.add(password);
    let mut group = node("AXGroup", "");
    group.children = vec![seven, field, password];
    let group = d.add(group);
    d.window(root, "Calculator", vec![group]);
    let mut export = node("AXMenuItem", "Export…");
    export.menu_key = Some("E".to_owned());
    let export = d.add(export);
    let mut items = node("AXMenu", "");
    items.children = vec![export];
    let items = d.add(items);
    let mut file_menu = node("AXMenuBarItem", "File");
    file_menu.children = vec![items];
    let file_menu = d.add(file_menu);
    let apple = d.add(node("AXMenuBarItem", "Apple"));
    let mut bar = node("AXMenuBar", "");
    bar.children = vec![apple, file_menu];
    let bar = d.add(bar);
    d.edit(root, |app| app.menu_bar = Some(bar));
    Calc {
        seven,
        field,
        password,
        file_menu,
        export,
    }
}

fn snapshot(rig: &Rig) -> String {
    let reply = rig
        .hands
        .handle("/snapshot", &body(json!({ "app": "Calculator" })))
        .unwrap();
    text(&reply).to_owned()
}

#[test]
fn a_snapshot_lists_elements_with_refs_and_the_menus() {
    let rig = rig();
    calculator(&rig);
    assert_eq!(
        snapshot(&rig),
        "Calculator (pid 10, in the background)\n\n\
         [a1] window \"Calculator\"\n\
         \x20 [a2] button \"7\"\n\
         \x20 [a3] text field \"Display\" = \"42\"\n\
         \x20 [a4] text field \"Code\" (password: never read or typed; the owner types it)\n\
         menus: File (mac_app_menu \"File\" lists one, \"File > …\" picks an item)"
    );
}

#[test]
fn a_password_field_s_value_is_never_read() {
    let rig = rig();
    let calc = calculator(&rig);
    let snapshot = snapshot(&rig);
    assert!(!snapshot.contains("hunter2"));
    let describe = rig
        .hands
        .handle(
            "/describe",
            &body(json!({ "app": "Calculator", "ref": "a4" })),
        )
        .unwrap();
    assert_eq!(describe.get("secret"), Some(&json!(true)));
    assert!(
        !rig.desktop
            .value_reads
            .lock()
            .unwrap()
            .contains(&calc.password),
        "a password field's value was read"
    );
}

#[test]
fn a_password_field_is_never_set_or_typed_into() {
    let rig = rig();
    calculator(&rig);
    snapshot(&rig);
    for path in ["/set", "/keys"] {
        let refused = rig
            .hands
            .handle(
                path,
                &body(json!({ "app": "Calculator", "ref": "a4", "text": "hunter3" })),
            )
            .unwrap_err();
        assert_eq!(refused.status, 403, "{path}");
        assert!(refused.text.contains("Hand it to the owner"), "{path}");
    }
    assert!(rig.desktop.set.lock().unwrap().is_empty());
    assert!(rig.desktop.posted.lock().unwrap().is_empty());
}

#[test]
fn keys_are_refused_while_a_password_field_has_the_focus() {
    let rig = rig();
    let calc = calculator(&rig);
    rig.desktop.edit(rig.desktop.application(10), |app| {
        app.focus = Some(calc.password);
    });
    let refused = rig
        .hands
        .handle("/keys", &body(json!({ "app": "Calculator", "text": "x" })))
        .unwrap_err();
    assert_eq!(refused.status, 403);
    assert!(
        refused
            .text
            .starts_with("Calculator has a password field focused.")
    );
    assert!(rig.desktop.posted.lock().unwrap().is_empty());
}

#[test]
fn a_field_labelled_for_a_code_counts_as_a_password() {
    let rig = rig();
    let calc = calculator(&rig);
    rig.desktop.edit(calc.field, |field| {
        field.description = Some("Verification code".to_owned());
    });
    let shown = snapshot(&rig);
    assert!(!shown.contains("= \"42\""), "{shown}");
    let refused = rig
        .hands
        .handle(
            "/set",
            &body(json!({ "app": "Calculator", "ref": "a3", "text": "123456" })),
        )
        .unwrap_err();
    assert_eq!(refused.status, 403);
}

#[test]
fn a_field_is_set_and_read_back() {
    let rig = rig();
    let calc = calculator(&rig);
    snapshot(&rig);
    let reply = rig
        .hands
        .handle(
            "/set",
            &body(json!({ "app": "Calculator", "ref": "a3", "text": "7" })),
        )
        .unwrap();
    assert_eq!(
        text(&reply),
        "Set text field \"Display\" in Calculator to \"7\"."
    );
    assert_eq!(
        *rig.desktop.set.lock().unwrap(),
        [(calc.field, "7".to_owned())]
    );
}

#[test]
fn a_field_that_can_t_be_set_says_so_in_the_words_the_server_falls_back_on() {
    let rig = rig();
    let calc = calculator(&rig);
    rig.desktop.edit(calc.field, |field| field.settable = false);
    snapshot(&rig);
    let refused = rig
        .hands
        .handle(
            "/set",
            &body(json!({ "app": "Calculator", "ref": "a3", "text": "7" })),
        )
        .unwrap_err();
    assert!(refused.text.contains("can't be set directly"));
}

#[test]
fn a_press_by_ref_or_label_and_the_front_app_put_back() {
    let rig = rig();
    let calc = calculator(&rig);
    rig.desktop.run(20, "Notes", "com.apple.Notes");
    *rig.desktop.front.lock().unwrap() = Some(20);
    *rig.desktop.jumps_front.lock().unwrap() = Some(10);
    let none = rig
        .hands
        .handle("/press", &body(json!({ "app": "Calculator", "ref": "a1" })))
        .unwrap_err();
    assert!(none.text.contains("No snapshot of this app yet"));
    snapshot(&rig);
    let reply = rig
        .hands
        .handle("/press", &body(json!({ "app": "Calculator", "ref": "7" })))
        .unwrap();
    assert_eq!(
        text(&reply),
        "Pressed button \"7\" in Calculator (Calculator came to the front; Notes was put back in \
         front). Take a snapshot to see the result."
    );
    assert_eq!(rig.desktop.front(), Some(20));
    assert_eq!(
        *rig.desktop.performed.lock().unwrap(),
        [(calc.seven, "AXPress".to_owned())]
    );
    let missing = rig
        .hands
        .handle("/press", &body(json!({ "app": "Calculator", "ref": "a9" })))
        .unwrap_err();
    assert_eq!(
        missing.text,
        "No element a9 in this app's last snapshot: take a new one."
    );
}

#[test]
fn keys_go_to_the_app_only() {
    let rig = rig();
    calculator(&rig);
    let reply = rig
        .hands
        .handle(
            "/keys",
            &body(json!({ "app": "Calculator", "text": "9", "key": "Return" })),
        )
        .unwrap();
    assert!(text(&reply).starts_with("Sent to Calculator only: typed \"9\", pressed Return."));
    let posted = rig.desktop.posted.lock().unwrap();
    assert_eq!(posted.len(), 2);
    assert!(posted.iter().all(|(to, _)| *to == Some(10)));
}

#[test]
fn a_menu_is_listed_and_an_item_picked() {
    let rig = rig();
    let calc = calculator(&rig);
    let listed = rig
        .hands
        .handle(
            "/menu",
            &body(json!({ "app": "Calculator", "path": "file" })),
        )
        .unwrap();
    assert_eq!(text(&listed), "Calculator › File:\n- Export… ⌘E");
    let picked = rig
        .hands
        .handle(
            "/menu",
            &body(json!({ "app": "Calculator", "path": "File > Export" })),
        )
        .unwrap();
    assert!(text(&picked).starts_with("Picked File > Export… in Calculator"));
    assert_eq!(
        *rig.desktop.performed.lock().unwrap(),
        [(calc.export, "AXPress".to_owned())]
    );
    let apple = rig
        .hands
        .handle(
            "/menu",
            &body(json!({ "app": "Calculator", "path": "Apple" })),
        )
        .unwrap_err();
    assert!(
        apple
            .text
            .starts_with("No \"Apple\" in Calculator's menu bar")
    );
    let _ = calc.file_menu;
}

#[test]
fn chrome_is_refused_however_it_is_named() {
    let rig = rig();
    rig.desktop.run(30, "Google Chrome", "com.google.Chrome");
    rig.desktop.installed.lock().unwrap().push(InstalledApp {
        path: "/Applications/Brave Browser.app".to_owned(),
        bundle: "com.brave.Browser".to_owned(),
        name: "Brave Browser".to_owned(),
    });
    for (path, app) in [
        ("/snapshot", "Google Chrome"),
        ("/snapshot", "30"),
        ("/which", "com.google.chrome"),
        ("/keys", "Google Chrome"),
        ("/open", "Brave Browser"),
        ("/which", "com.brave.Browser"),
    ] {
        let refused = rig
            .hands
            .handle(path, &body(json!({ "app": app })))
            .unwrap_err();
        assert_eq!(refused.status, 403, "{path} {app}");
        assert!(
            refused
                .text
                .starts_with("Chrome goes through the browser extension")
        );
    }
    assert!(rig.desktop.opened.lock().unwrap().is_empty());
}

#[test]
fn the_never_list_and_terminal_s_look_only_rule_hold() {
    let rig = rig();
    let root = rig.desktop.run(40, "Terminal", "com.apple.Terminal");
    rig.desktop.window(root, "zsh", vec![]);
    rig.desktop
        .run(41, "Keychain Access", "com.apple.keychainaccess");
    assert!(
        rig.hands
            .handle("/snapshot", &body(json!({ "app": "Terminal" })))
            .is_ok()
    );
    let pressed = rig
        .hands
        .handle("/keys", &body(json!({ "app": "Terminal", "text": "ls" })))
        .unwrap_err();
    assert!(pressed.text.starts_with("Terminal is only looked at"));
    let keychain = rig
        .hands
        .handle("/snapshot", &body(json!({ "app": "Keychain Access" })))
        .unwrap_err();
    assert_eq!(keychain.status, 403);
    let listed = rig.hands.handle("/apps", &Reply::new()).unwrap();
    assert!(text(&listed).contains(
        "- Keychain Access (com.apple.keychainaccess, pid 41, never used by the assistant): no windows"
    ));
    assert!(text(&listed).contains("- Terminal (com.apple.Terminal, pid 40): \"zsh\""));
}

#[test]
fn one_app_at_a_time() {
    let rig = rig();
    calculator(&rig);
    let notes = rig.desktop.run(20, "Notes", "com.apple.Notes");
    rig.desktop.window(notes, "Notes", vec![]);
    snapshot(&rig);
    let refused = rig
        .hands
        .handle("/snapshot", &body(json!({ "app": "Notes" })))
        .unwrap_err();
    assert_eq!(refused.status, 409);
    assert!(refused.text.contains("using Calculator in the background"));
    // /which only names an app; it doesn't take it.
    assert!(
        rig.hands
            .handle("/which", &body(json!({ "app": "Notes" })))
            .is_ok()
    );
    rig.clock.advance(HOLD);
    assert!(
        rig.hands
            .handle("/snapshot", &body(json!({ "app": "Notes" })))
            .is_ok()
    );
    // Calculator's refs went with it.
    let gone = rig
        .hands
        .handle("/press", &body(json!({ "app": "Notes", "ref": "a2" })))
        .unwrap_err();
    assert!(gone.text.starts_with("No element a2"), "{}", gone.text);
}

#[test]
fn release_or_a_quit_lets_the_held_app_go() {
    let rig = rig();
    calculator(&rig);
    let notes = rig.desktop.run(20, "Notes", "com.apple.Notes");
    rig.desktop.window(notes, "Notes", vec![]);
    snapshot(&rig);
    let released = rig.hands.handle("/release", &Reply::new()).unwrap();
    assert_eq!(text(&released), "Let go of Calculator.");
    assert!(
        rig.hands
            .handle("/snapshot", &body(json!({ "app": "Notes" })))
            .is_ok()
    );
    rig.desktop.quit(20);
    snapshot(&rig);
}

#[test]
fn which_names_an_app_on_disk_and_open_opens_it_hidden() {
    let rig = rig();
    rig.desktop.installed.lock().unwrap().push(InstalledApp {
        path: "/Applications/Pages.app".to_owned(),
        bundle: "com.apple.iWork.Pages".to_owned(),
        name: "Pages".to_owned(),
    });
    let which = rig
        .hands
        .handle("/which", &body(json!({ "app": "pages" })))
        .unwrap();
    assert_eq!(
        Value::Object(which),
        json!({ "name": "Pages", "bundle": "com.apple.iWork.Pages", "running": false })
    );
    let opened = rig
        .hands
        .handle("/open", &body(json!({ "app": "Pages" })))
        .unwrap();
    assert_eq!(
        text(&opened),
        "Opened Pages in the background, hidden. Take mac_app_snapshot to read it."
    );
    assert_eq!(
        *rig.desktop.opened.lock().unwrap(),
        [("Pages".to_owned(), true)]
    );
    let missing = rig
        .hands
        .handle("/which", &body(json!({ "app": "Nope" })))
        .unwrap_err();
    assert_eq!(missing.status, 404);
}

#[test]
fn without_accessibility_the_owner_is_told_where_to_turn_it_on() {
    let rig = rig();
    calculator(&rig);
    rig.desktop.accessibility.store(false, Ordering::SeqCst);
    let refused = rig
        .hands
        .handle("/snapshot", &body(json!({ "app": "Calculator" })))
        .unwrap_err();
    assert_eq!(refused.status, 403);
    assert!(refused.text.contains("Privacy & Security → Accessibility"));
}

#[test]
fn the_whole_screen_shows_the_bar_and_stop_ends_it_at_once() {
    let rig = rig();
    let me = json!({ "who": "agent" });
    let early = rig
        .hands
        .handle("/screen/act", &body(me.clone()))
        .unwrap_err();
    assert_eq!(early.status, 409);
    let started = rig
        .hands
        .handle(
            "/screen/start",
            &body(json!({ "who": "agent", "reason": "fill the form", "minutes": 5 })),
        )
        .unwrap();
    assert!(text(&started).starts_with("Using the owner's whole screen for up to 5 minutes"));
    assert_eq!(
        rig.desktop
            .bars
            .lock()
            .unwrap()
            .last()
            .cloned()
            .flatten()
            .as_deref(),
        Some("Emberhut is using your screen: fill the form")
    );
    rig.desktop.run(50, "Notes", "com.apple.Notes");
    *rig.desktop.under_pointer.lock().unwrap() = Some(50);
    let shot = rig.hands.handle("/screen/shot", &body(me.clone())).unwrap();
    assert!(text(&shot).contains("1512×982 points"));
    assert!(shot.get("image").is_some());
    let clicked = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "click", "x": 10, "y": 20 })),
        )
        .unwrap();
    assert_eq!(
        text(&clicked),
        "Done: click at 10,20. mac_screen shot to see it."
    );
    {
        let posted = rig.desktop.posted.lock().unwrap();
        let (to, inputs) = posted.last().unwrap();
        assert_eq!(*to, None);
        assert!(inputs.contains(&Input::Click {
            button: Button::Left,
            down: true,
            x: 10.0,
            y: 20.0,
            count: 1
        }));
    }
    rig.hands.owner_stopped();
    assert_eq!(rig.desktop.bars.lock().unwrap().last(), Some(&None));
    let after = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "click", "x": 1, "y": 1 })),
        )
        .unwrap_err();
    assert_eq!(after.status, 409);
    assert!(after.text.starts_with("The owner pressed Stop on the bar"));
    let health = rig.hands.handle("/health", &Reply::new()).unwrap();
    assert_eq!(health.get("driving"), Some(&Value::Null));
    assert_eq!(health.get("stopped"), Some(&json!(0.0)));
}

#[test]
fn the_owner_s_touch_ends_the_screen_session() {
    let rig = rig();
    rig.hands
        .handle(
            "/screen/start",
            &body(json!({ "who": "agent", "reason": "x" })),
        )
        .unwrap();
    *rig.desktop.idle.lock().unwrap() = 0.1;
    rig.hands.watch();
    assert_eq!(rig.desktop.bars.lock().unwrap().last(), Some(&None));
    let after = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "key", "key": "tab" })),
        )
        .unwrap_err();
    assert!(
        after
            .text
            .starts_with("The owner used the mouse or keyboard")
    );
}

#[test]
fn typing_on_the_screen_into_a_password_field_is_refused() {
    let rig = rig();
    let calc = calculator(&rig);
    *rig.desktop.system_focus.lock().unwrap() = Focus::On(calc.password);
    rig.hands
        .handle(
            "/screen/start",
            &body(json!({ "who": "agent", "reason": "x" })),
        )
        .unwrap();
    let refused = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "type", "text": "hunter2" })),
        )
        .unwrap_err();
    assert_eq!(refused.status, 403);
    assert!(rig.desktop.posted.lock().unwrap().is_empty());
}

#[test]
fn the_screen_needs_both_grants() {
    let rig = rig();
    rig.desktop.screen.store(false, Ordering::SeqCst);
    let refused = rig
        .hands
        .handle("/screen/start", &body(json!({ "who": "agent" })))
        .unwrap_err();
    assert_eq!(refused.status, 403);
}

#[test]
fn health_says_the_grants_and_what_runs() {
    let rig = rig();
    let health = rig.hands.handle("/health", &Reply::new()).unwrap();
    assert_eq!(
        Value::Object(health),
        json!({
            "ok": true,
            "accessibility": true,
            "screen": true,
            "driving": null,
            "recording": null,
            "idle": 600.0,
            "locked": false,
            "stopped": null,
        })
    );
    let locked = rig.hands.handle("/lock", &Reply::new()).unwrap();
    assert_eq!(locked.get("locked"), Some(&json!(true)));
}

#[test]
fn recording_shows_the_bar_and_stop_on_it_ends_the_recording() {
    let rig = rig();
    rig.hands
        .handle(
            "/record/start",
            &body(json!({ "id": "2026-11-26-abc123", "title": "Weekly" })),
        )
        .unwrap();
    assert_eq!(
        rig.desktop
            .bars
            .lock()
            .unwrap()
            .last()
            .cloned()
            .flatten()
            .as_deref(),
        Some("Emberhut is recording: Weekly")
    );
    rig.hands.owner_stopped();
    assert!(rig.audio.finished.load(Ordering::SeqCst));
    assert_eq!(rig.hands.recorder().current(), None);
    assert_eq!(rig.desktop.bars.lock().unwrap().last(), Some(&None));
}

#[test]
fn an_unknown_path_is_not_found() {
    let rig = rig();
    let missing = rig.hands.handle("/chrome/open", &Reply::new()).unwrap_err();
    assert_eq!(missing.status, 404);
    assert_eq!(missing.text, "no /chrome/open");
}

/// Starts a screen session for `agent`.
fn start_screen(rig: &Rig) {
    rig.hands
        .handle(
            "/screen/start",
            &body(json!({ "who": "agent", "reason": "x" })),
        )
        .unwrap();
}

#[test]
fn stop_on_the_bar_stops_typing_in_the_middle() {
    let rig = rig();
    start_screen(&rig);
    let hands = Arc::downgrade(&rig.hands);
    *rig.desktop.on_post.lock().unwrap() = Some(Box::new(move |count| {
        if count == 4
            && let Some(hands) = hands.upgrade()
        {
            hands.owner_stopped();
        }
    }));
    let refused = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "type", "text": "abcdefgh" })),
        )
        .unwrap_err();
    assert_eq!(refused.status, 409);
    assert!(
        refused.text.starts_with("The owner pressed Stop"),
        "{}",
        refused.text
    );
    let posted: usize = rig
        .desktop
        .posted
        .lock()
        .unwrap()
        .iter()
        .map(|(_, inputs)| inputs.len())
        .sum();
    assert_eq!(posted, 4, "events went on after Stop");
}

#[test]
fn the_owner_s_touch_while_typing_stops_the_next_event() {
    let rig = rig();
    start_screen(&rig);
    let desktop = Arc::downgrade(&rig.desktop);
    *rig.desktop.on_post.lock().unwrap() = Some(Box::new(move |count| {
        if count == 2
            && let Some(desktop) = desktop.upgrade()
        {
            // The owner presses a key right after the assistant's second event.
            desktop.input_count.fetch_add(1, Ordering::SeqCst);
        }
    }));
    let refused = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "type", "text": "abcdefgh" })),
        )
        .unwrap_err();
    assert!(
        refused.text.starts_with("The owner is using the Mac"),
        "{}",
        refused.text
    );
    let health = rig.hands.handle("/health", &Reply::new()).unwrap();
    assert_eq!(health.get("driving"), Some(&Value::Null));
}

#[test]
fn the_assistant_s_own_typing_never_counts_as_a_touch() {
    let rig = rig();
    start_screen(&rig);
    // Each of its events is counted by the Mac and makes the Mac's idle time 0.
    for _ in 0..3 {
        rig.hands
            .handle(
                "/screen/act",
                &body(json!({ "who": "agent", "action": "type", "text": "abcdefgh" })),
            )
            .unwrap();
        rig.hands.watch();
    }
    let health = rig.hands.handle("/health", &Reply::new()).unwrap();
    assert_eq!(health.get("driving"), Some(&json!("agent")));
}

#[test]
fn too_much_text_is_refused() {
    let rig = rig();
    calculator(&rig);
    let long = "a".repeat(crate::keys::MAX_TEXT + 1);
    let refused = rig
        .hands
        .handle("/keys", &body(json!({ "app": "Calculator", "text": long })))
        .unwrap_err();
    assert_eq!(refused.status, 400);
    start_screen(&rig);
    let refused = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "type", "text": long })),
        )
        .unwrap_err();
    assert_eq!(refused.status, 400);
    assert!(rig.desktop.posted.lock().unwrap().is_empty());
}

#[test]
fn a_tab_into_a_password_field_stops_the_typing_there() {
    let rig = rig();
    let calc = calculator(&rig);
    let root = rig.desktop.application(10);
    let desktop = Arc::downgrade(&rig.desktop);
    *rig.desktop.on_post.lock().unwrap() = Some(Box::new(move |_| {
        if let Some(desktop) = desktop.upgrade() {
            // The Tab moved the focus to the password field.
            desktop.edit(root, |app| app.focus = Some(calc.password));
        }
    }));
    let refused = rig
        .hands
        .handle(
            "/keys",
            &body(json!({ "app": "Calculator", "text": "user\thunter2" })),
        )
        .unwrap_err();
    assert_eq!(refused.status, 403);
    let typed: Vec<Input> = rig
        .desktop
        .posted
        .lock()
        .unwrap()
        .iter()
        .flat_map(|(_, inputs)| inputs.clone())
        .collect();
    let units: Vec<u16> = typed
        .iter()
        .filter_map(|input| match input {
            Input::Text { units, down: true } => units.first().copied(),
            _ => None,
        })
        .collect();
    assert_eq!(String::from_utf16_lossy(&units), "user\t");
}

#[test]
fn typing_where_the_focus_can_t_be_told_is_refused() {
    let rig = rig();
    calculator(&rig);
    rig.desktop.focus_unknown.store(true, Ordering::SeqCst);
    let refused = rig
        .hands
        .handle("/keys", &body(json!({ "app": "Calculator", "text": "9" })))
        .unwrap_err();
    assert_eq!(refused.status, 403);
    assert!(
        refused
            .text
            .contains("doesn't say what has the keyboard focus")
    );
    *rig.desktop.system_focus.lock().unwrap() = Focus::Unknown;
    start_screen(&rig);
    let screen = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "type", "text": "9" })),
        )
        .unwrap_err();
    assert_eq!(screen.status, 403);
}

#[test]
fn on_the_screen_return_into_a_password_field_is_refused_but_tab_goes() {
    let rig = rig();
    let calc = calculator(&rig);
    *rig.desktop.system_focus.lock().unwrap() = Focus::On(calc.password);
    start_screen(&rig);
    let act = |key: &str| {
        rig.hands.handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "key", "key": key })),
        )
    };
    assert_eq!(act("Return").unwrap_err().status, 403);
    assert!(act("shift+tab").is_ok());
}

#[test]
fn the_screen_keeps_the_app_rules() {
    let rig = rig();
    rig.desktop
        .run(41, "Keychain Access", "com.apple.keychainaccess");
    rig.desktop.run(30, "Google Chrome", "com.google.Chrome");
    rig.desktop.run(50, "Notes", "com.apple.Notes");
    start_screen(&rig);
    *rig.desktop.front.lock().unwrap() = Some(41);
    let refused = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "key", "key": "tab" })),
        )
        .unwrap_err();
    assert_eq!(refused.status, 403);
    *rig.desktop.front.lock().unwrap() = Some(50);
    *rig.desktop.under_pointer.lock().unwrap() = Some(30);
    let click = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "click", "x": 5, "y": 5 })),
        )
        .unwrap_err();
    assert!(
        click
            .text
            .starts_with("Chrome goes through the browser extension")
    );
    *rig.desktop.under_pointer.lock().unwrap() = None;
    let unknown = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "click", "x": 5, "y": 5 })),
        )
        .unwrap_err();
    assert!(unknown.text.contains("doesn't say which app is under 5,5"));
    *rig.desktop.under_pointer.lock().unwrap() = Some(50);
    assert!(
        rig.hands
            .handle(
                "/screen/act",
                &body(json!({ "who": "agent", "action": "click", "x": 5, "y": 5 })),
            )
            .is_ok()
    );
}

#[test]
fn a_new_screen_start_waits_after_stop_and_never_while_locked() {
    let rig = rig();
    start_screen(&rig);
    rig.hands.owner_stopped();
    let early = rig
        .hands
        .handle("/screen/start", &body(json!({ "who": "agent" })))
        .unwrap_err();
    assert_eq!(early.status, 409);
    rig.clock.advance(Duration::from_secs(60));
    start_screen(&rig);
    rig.hands
        .handle("/screen/stop", &body(json!({ "who": "agent" })))
        .unwrap();
    rig.desktop.locked.store(true, Ordering::SeqCst);
    let locked = rig
        .hands
        .handle("/screen/start", &body(json!({ "who": "agent" })))
        .unwrap_err();
    assert!(locked.text.contains("locked"));
}

#[test]
fn without_the_bar_the_screen_is_not_used() {
    let rig = rig();
    rig.desktop.bar_fails.store(true, Ordering::SeqCst);
    let refused = rig
        .hands
        .handle("/screen/start", &body(json!({ "who": "agent" })))
        .unwrap_err();
    assert_eq!(refused.status, 500);
    let health = rig.hands.handle("/health", &Reply::new()).unwrap();
    assert_eq!(health.get("driving"), Some(&Value::Null));
}

#[test]
fn a_lost_capture_ends_the_recording_and_its_bar() {
    let rig = rig();
    rig.hands
        .handle("/record/start", &body(json!({ "id": "abcdef1" })))
        .unwrap();
    // A capture of an older recording lost late leaves this one recording.
    rig.hands.capture_lost(0);
    assert!(rig.hands.recorder().current().is_some());
    rig.hands.capture_lost(1);
    assert_eq!(rig.hands.recorder().current(), None);
    assert_eq!(rig.desktop.bars.lock().unwrap().last(), Some(&None));
}

#[test]
fn without_the_bar_nothing_is_recorded() {
    let rig = rig();
    rig.desktop.bar_fails.store(true, Ordering::SeqCst);
    let refused = rig
        .hands
        .handle("/record/start", &body(json!({ "id": "abcdef1" })))
        .unwrap_err();
    assert_eq!(refused.status, 500);
    assert_eq!(rig.hands.recorder().current(), None);
    assert!(rig.audio.finished.load(Ordering::SeqCst));
}

#[test]
fn an_app_found_by_its_bundle_stays_held_while_it_runs() {
    let rig = rig();
    calculator(&rig);
    let notes = rig.desktop.run(20, "Notes", "com.apple.Notes");
    rig.desktop.window(notes, "Notes", vec![]);
    snapshot(&rig);
    // Calculator leaves the Dock's list but still runs: Notes waits.
    let calculator_app = rig.desktop.app(10).unwrap();
    rig.desktop.quit(10);
    rig.desktop.background.lock().unwrap().push(calculator_app);
    assert!(
        rig.hands
            .handle("/snapshot", &body(json!({ "app": "Notes" })))
            .is_err()
    );
}

#[test]
fn after_a_tab_that_doesn_t_move_the_focus_the_rest_isn_t_typed() {
    let rig = rig();
    let calc = calculator(&rig);
    let root = rig.desktop.application(10);
    rig.desktop.edit(root, |app| app.focus = Some(calc.field));
    let refused = rig
        .hands
        .handle(
            "/keys",
            &body(json!({ "app": "Calculator", "text": "user\tsecret" })),
        )
        .unwrap_err();
    assert_eq!(refused.status, 403);
    assert!(
        refused.text.starts_with("After the Tab"),
        "{}",
        refused.text
    );
    let typed: usize = rig
        .desktop
        .posted
        .lock()
        .unwrap()
        .iter()
        .map(|(_, inputs)| inputs.len())
        .sum();
    // "user\t": five characters, down and up.
    assert_eq!(typed, 10);
}

#[test]
fn after_a_tab_that_leaves_nothing_focused_the_rest_isn_t_typed() {
    let rig = rig();
    let calc = calculator(&rig);
    let root = rig.desktop.application(10);
    rig.desktop.edit(root, |app| app.focus = Some(calc.field));
    let desktop = Arc::downgrade(&rig.desktop);
    *rig.desktop.on_post.lock().unwrap() = Some(Box::new(move |_| {
        if let Some(desktop) = desktop.upgrade() {
            desktop.edit(root, |app| app.focus = None);
        }
    }));
    let refused = rig
        .hands
        .handle(
            "/keys",
            &body(json!({ "app": "Calculator", "text": "user\tsecret" })),
        )
        .unwrap_err();
    assert!(
        refused.text.starts_with("After the Tab"),
        "{}",
        refused.text
    );
}

#[test]
fn a_new_line_in_a_text_area_stays_there() {
    let rig = rig();
    let calc = calculator(&rig);
    rig.desktop
        .edit(calc.field, |field| field.role = "AXTextArea".to_owned());
    let root = rig.desktop.application(10);
    rig.desktop.edit(root, |app| app.focus = Some(calc.field));
    rig.hands
        .handle(
            "/keys",
            &body(json!({ "app": "Calculator", "text": "one\ntwo\nthree" })),
        )
        .unwrap();
    // On the whole screen, a Return in a one-line field that keeps the focus stops the rest.
    *rig.desktop.system_focus.lock().unwrap() = Focus::On(calc.seven);
    start_screen(&rig);
    let refused = rig
        .hands
        .handle(
            "/screen/act",
            &body(json!({ "who": "agent", "action": "type", "text": "query\rmore" })),
        )
        .unwrap_err();
    assert!(
        refused.text.starts_with("After the Return"),
        "{}",
        refused.text
    );
}
