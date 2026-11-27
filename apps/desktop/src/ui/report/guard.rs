//! The script that runs in a report before any of the page's own (a Wry initialization script,
//! in every frame): it takes away what a report has no business using, and turns the user's
//! link clicks into `window.open`, which `WebKit` allows only during a click (see
//! [`super::policy`]). It is a page script and nothing more: it can't reach the app.
//!
//! - No camera, microphone, screen capture, location, notifications, credentials, peer
//!   connections or recording. Wry 0.53 grants every media capture request without asking, and
//!   has no handler to refuse one, so the APIs are removed before the page can call them.
//!   Upstream Wry 0.57's permission handler is the later fix (ADR 0006).
//! - No file picker: a click on `<input type="file">` (the user's, a label's or a script's) is
//!   cancelled and `showPicker` is removed. Wry's file panel would otherwise hand the page any
//!   file the user picks.
//! - A script's click on a link is cancelled. The user's plain click on an http, https or mailto
//!   link (HTML or SVG) opens it through `window.open` instead of navigating the page.

/// The script.
pub(super) const GUARD: &str = r"(() => {
  'use strict';
  const remove = (owner, name) => {
    if (!owner) return;
    try {
      delete owner[name];
      Object.defineProperty(owner, name, { value: undefined, configurable: false, writable: false });
    } catch (_) {}
  };
  const navigatorProto = window.Navigator && Navigator.prototype;
  for (const name of ['mediaDevices', 'getUserMedia', 'webkitGetUserMedia', 'geolocation', 'credentials']) {
    remove(navigatorProto, name);
  }
  const mediaProto = window.MediaDevices && MediaDevices.prototype;
  for (const name of ['getUserMedia', 'getDisplayMedia', 'enumerateDevices']) {
    remove(mediaProto, name);
  }
  if (window.Notification) remove(window.Notification, 'requestPermission');
  for (const name of ['Notification', 'RTCPeerConnection', 'webkitRTCPeerConnection', 'MediaRecorder',
    'showOpenFilePicker', 'showSaveFilePicker', 'showDirectoryPicker']) {
    remove(window, name);
  }
  remove(window.HTMLInputElement && HTMLInputElement.prototype, 'showPicker');

  const open = window.open.bind(window);
  const find = (event, test) => event.composedPath().find(test);
  const isFileInput = (node) => node instanceof HTMLInputElement && node.type === 'file';
  const isLink = (node) => node instanceof HTMLAnchorElement || node instanceof HTMLAreaElement ||
    node instanceof SVGAElement;
  const target = (link) => {
    try {
      return new URL(link instanceof SVGAElement ? link.href.baseVal : link.href, document.baseURI);
    } catch (_) {
      return null;
    }
  };
  const opened = ['http:', 'https:', 'mailto:'];
  window.addEventListener('click', (event) => {
    if (find(event, isFileInput)) {
      event.preventDefault();
      event.stopImmediatePropagation();
    } else if (!event.isTrusted && find(event, isLink)) {
      event.preventDefault();
    }
  }, true);
  window.addEventListener('click', (event) => {
    if (!event.isTrusted || event.defaultPrevented || event.button !== 0) return;
    const link = find(event, isLink);
    const url = link && target(link);
    if (!url || !opened.includes(url.protocol)) return;
    event.preventDefault();
    open(url.href);
  });
})();
";

#[cfg(test)]
mod tests {
    use super::GUARD;

    #[test]
    fn hardware_and_prompts_are_taken_away() {
        for name in [
            "'mediaDevices'",
            "'getUserMedia'",
            "'webkitGetUserMedia'",
            "'getDisplayMedia'",
            "'geolocation'",
            "'credentials'",
            "'requestPermission'",
            "'Notification'",
            "'RTCPeerConnection'",
            "'webkitRTCPeerConnection'",
            "'MediaRecorder'",
        ] {
            assert!(GUARD.contains(name), "{name}");
        }
        // Removed for good: not writable, not configurable, so the page can't put it back.
        assert!(GUARD.contains("configurable: false, writable: false"));
    }

    #[test]
    fn file_pickers_are_refused() {
        assert!(GUARD.contains("node.type === 'file'"));
        assert!(GUARD.contains("'showPicker'"));
        assert!(GUARD.contains("event.stopImmediatePropagation()"));
    }

    #[test]
    fn only_the_users_clicks_open_links_and_through_window_open() {
        // A script's click on a link is cancelled, in the capture phase (before the page's own
        // listeners); the user's plain click on a web link becomes `window.open`, taken before
        // the page could replace it.
        assert!(GUARD.contains("!event.isTrusted && find(event, isLink)"));
        assert!(GUARD.contains("}, true);"));
        assert!(GUARD.contains("const open = window.open.bind(window);"));
        assert!(GUARD.contains("open(url.href);"));
        assert!(GUARD.contains("const opened = ['http:', 'https:', 'mailto:'];"));
        assert!(GUARD.contains("!opened.includes(url.protocol)"));
    }

    #[test]
    fn svg_links_are_links_too() {
        // Links in SVG charts: their `href` is an SVGAnimatedString, resolved before use.
        assert!(GUARD.contains("node instanceof SVGAElement"));
        assert!(GUARD.contains("link.href.baseVal"));
    }

    #[test]
    fn the_script_is_one_closed_strict_function() {
        assert!(GUARD.starts_with("(() => {\n  'use strict';"));
        assert!(GUARD.trim_end().ends_with("})();"));
        let mut depth = Vec::new();
        for c in GUARD.chars() {
            match c {
                '(' | '[' | '{' => depth.push(c),
                ')' | ']' | '}' => {
                    let open = depth.pop();
                    let matching = match c {
                        ')' => '(',
                        ']' => '[',
                        _ => '{',
                    };
                    assert_eq!(open, Some(matching), "unbalanced {c}");
                }
                _ => {}
            }
        }
        assert!(depth.is_empty(), "unclosed {depth:?}");
        // Nothing that would reach out of the page.
        for word in ["fetch(", "XMLHttpRequest", "webkit.messageHandlers", "ipc"] {
            assert!(!GUARD.contains(word), "{word}");
        }
    }
}
