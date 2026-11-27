//! The Content-Security-Policy a report runs under in the web view, put ahead of everything in
//! the page: the reference client's policy, word for word. After it, the page's canvas in the
//! app's surface and text, for a report that sets no background or text colour of its own.

use crate::ui::palette::WebColours;

/// What the page may load: its own inline styles, scripts, fonts and media, and pictures inline
/// or over https (never plain http: a page can't reach a server on the local network); no
/// requests, forms, frames or base address.
pub(super) const POLICY: &str = "default-src 'none'; img-src https: data: blob:; \
    media-src data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; \
    font-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-src 'none'";

/// `html` with the policy as its very first element: right after its doctype (so the page
/// stays in standards mode), before anything of its own can run. Then the canvas: `colours`
/// (light, dark) as the root's background and text in the web view's appearance, so a report
/// without colours of its own sits on the panel's surface in its text rather than in
/// `WebKit`'s white and black (black on the dark surface would hardly show). `:where` gives
/// it no weight: any background or colour of the page's own wins.
#[must_use]
pub(super) fn sandboxed(html: &str, colours: &[WebColours; 2]) -> String {
    let meta = format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{POLICY}\">");
    let canvas = canvas(colours);
    let at = doctype_end(html).unwrap_or(0);
    let mut page = String::with_capacity(html.len() + meta.len() + canvas.len());
    page.push_str(&html[..at]);
    page.push_str(&meta);
    page.push_str(&canvas);
    page.push_str(&html[at..]);
    page
}

/// The canvas's style for `colours` (light, dark). An appearance whose surface or text isn't
/// `#RRGGBB` is left out whole, so nothing but a colour reaches the page and no surface comes
/// without its text.
fn canvas(colours: &[WebColours; 2]) -> String {
    fn hex(colour: &str) -> Option<&str> {
        let digits = colour.strip_prefix('#')?;
        (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit())).then_some(digits)
    }
    let rule = |colours: &WebColours| {
        let (surface, text) = (hex(&colours.surface)?, hex(&colours.text)?);
        Some(format!(
            ":where(:root){{background-color:#{surface};color:#{text}}}"
        ))
    };
    let [light, dark] = colours;
    let mut style = String::from("<style>");
    style.extend(rule(light));
    if let Some(dark) = rule(dark) {
        style.push_str("@media (prefers-color-scheme: dark){");
        style.push_str(&dark);
        style.push('}');
    }
    style.push_str("</style>");
    style
}

/// Where the page's leading `<!doctype …>` (after any white space) ends, if it has one.
fn doctype_end(html: &str) -> Option<usize> {
    let start = html.len() - html.trim_start().len();
    let rest = &html[start..];
    let head = rest.get(..9)?;
    if !head.eq_ignore_ascii_case("<!doctype") {
        return None;
    }
    rest.find('>').map(|gt| start + gt + 1)
}

#[cfg(test)]
mod tests {
    use super::{POLICY, canvas, sandboxed};
    use crate::ui::palette::WebColours;

    /// Paper's surface and text, light and dark.
    fn colours() -> [WebColours; 2] {
        [("#FFFDF9", "#1D1712"), ("#211B16", "#F6EEE5")].map(|(surface, text)| WebColours {
            surface: surface.into(),
            text: text.into(),
        })
    }

    /// What goes ahead of the page: the policy, then the canvas.
    fn meta() -> String {
        format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"{POLICY}\">{}",
            canvas(&colours())
        )
    }

    #[test]
    fn the_policy_goes_right_after_the_doctype() {
        let page = "\n <!DOCTYPE html><html><script>run()</script></html>";
        assert_eq!(
            sandboxed(page, &colours()),
            format!(
                "\n <!DOCTYPE html>{}<html><script>run()</script></html>",
                meta()
            )
        );
    }

    #[test]
    fn without_a_doctype_the_policy_comes_first() {
        let page = "<html><head><title>T</title></head></html>";
        assert_eq!(sandboxed(page, &colours()), format!("{}{page}", meta()));
        // Not a doctype: a comment, a doctype later on, an unclosed one.
        for page in ["<!-- x --><!doctype html>", "<!doctype html", "<!doc"] {
            assert!(sandboxed(page, &colours()).starts_with(&meta()), "{page}");
        }
    }

    #[test]
    fn the_page_itself_is_kept_whole() {
        let page = "<!doctype html><p dir=\"rtl\">تقرير</p>";
        let out = sandboxed(page, &colours());
        assert_eq!(out.replacen(&meta(), "", 1), page);
    }

    #[test]
    fn the_policy_matches_the_reference_client() {
        // The reference client's policy, copied as one line.
        assert_eq!(
            POLICY,
            "default-src 'none'; img-src https: data: blob:; media-src data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; font-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-src 'none'"
        );
        assert!(!POLICY.contains("  "), "no doubled spaces");
    }

    #[test]
    fn the_policy_allows_no_requests_and_no_plain_http() {
        for directive in [
            "default-src 'none'",
            "connect-src 'none'",
            "frame-src 'none'",
            "form-action 'none'",
            "base-uri 'none'",
            "img-src https: data: blob:",
        ] {
            assert!(POLICY.contains(directive), "{directive}");
        }
        assert!(!POLICY.contains("http:"), "plain http is never allowed");
        assert!(
            !POLICY.contains('"'),
            "the policy sits in a quoted attribute"
        );
    }

    #[test]
    fn a_page_without_colours_sits_on_the_surface_in_the_text_of_its_appearance() {
        assert_eq!(
            canvas(&colours()),
            "<style>:where(:root){background-color:#FFFDF9;color:#1D1712}\
             @media (prefers-color-scheme: dark){\
             :where(:root){background-color:#211B16;color:#F6EEE5}}</style>"
        );
    }

    #[test]
    fn nothing_but_a_colour_reaches_the_page() {
        let [mut light, mut dark] = colours();
        light.surface = "red;}</style><script>".into();
        dark.text = "#12345".into();
        assert_eq!(canvas(&[light, dark]), "<style></style>");
    }
}
