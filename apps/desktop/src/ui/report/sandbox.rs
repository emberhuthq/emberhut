//! The Content-Security-Policy a report runs under in the web view, put ahead of everything in
//! the page: the reference client's policy, word for word.

/// What the page may load: its own inline styles, scripts, fonts and media, and pictures inline
/// or over https (never plain http: a page can't reach a server on the local network); no
/// requests, forms, frames or base address.
pub(super) const POLICY: &str = "default-src 'none'; img-src https: data: blob:; \
    media-src data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; \
    font-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-src 'none'";

/// `html` with the policy as its very first element: right after its doctype (so the page
/// stays in standards mode), before anything of its own can run.
#[must_use]
pub(super) fn sandboxed(html: &str) -> String {
    let meta = format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{POLICY}\">");
    let at = doctype_end(html).unwrap_or(0);
    let mut page = String::with_capacity(html.len() + meta.len());
    page.push_str(&html[..at]);
    page.push_str(&meta);
    page.push_str(&html[at..]);
    page
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
    use super::{POLICY, sandboxed};

    fn meta() -> String {
        format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{POLICY}\">")
    }

    #[test]
    fn the_policy_goes_right_after_the_doctype() {
        let page = "\n <!DOCTYPE html><html><script>run()</script></html>";
        assert_eq!(
            sandboxed(page),
            format!(
                "\n <!DOCTYPE html>{}<html><script>run()</script></html>",
                meta()
            )
        );
    }

    #[test]
    fn without_a_doctype_the_policy_comes_first() {
        let page = "<html><head><title>T</title></head></html>";
        assert_eq!(sandboxed(page), format!("{}{page}", meta()));
        // Not a doctype: a comment, a doctype later on, an unclosed one.
        for page in ["<!-- x --><!doctype html>", "<!doctype html", "<!doc"] {
            assert!(sandboxed(page).starts_with(&meta()), "{page}");
        }
    }

    #[test]
    fn the_page_itself_is_kept_whole() {
        let page = "<!doctype html><p dir=\"rtl\">تقرير</p>";
        let out = sandboxed(page);
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
}
