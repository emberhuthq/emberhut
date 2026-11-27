//! The browser tools (`browser_*`) and `chrome_pair`: the assistant's browser, or one of the
//! owner's Chrome profiles with `browser: "chrome:<profile>"`. Their logic is the browser
//! service's ([`crate::services::browser`]); these read the arguments.

use base64::Engine as _;
use emberhut_api::mcp::Content;
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::browser::surface::Who;
use crate::services::browser::{Act, BrowserArgs};

/// What every browser tool's description starts with.
macro_rules! intro {
    () => {
        "The assistant's own browser (a real Chrome on the server's machine, watched live in the \
         owner's app), or with browser \"chrome:<profile>\" one of the owner's own Chrome \
         profiles. "
    };
}

/// The answer as the agent gets it: its text, and its picture when it has one.
fn output(act: Act) -> ToolOutput {
    let mut content = Vec::new();
    if let Some(image) = act.image {
        content.push(Content::Image {
            data: base64::engine::general_purpose::STANDARD.encode(image),
            mime_type: "image/jpeg".to_owned(),
        });
    }
    content.push(Content::text(act.text));
    ToolOutput {
        content,
        is_error: false,
    }
}

/// One browser tool: its type, its arguments (with `task_id`, `browser` and `chrome` added),
/// its name and description, and how its arguments become the service's.
macro_rules! browser_tool {
    (
        $ty:ident, $args:ident, $name:literal, $desc:expr,
        { $($(#[$meta:meta])* $field:ident : $kind:ty),* $(,)? },
        |$a:ident| $fill:expr
    ) => {
        #[doc = concat!("`", $name, "`.")]
        #[derive(Clone, Copy, Debug)]
        pub(crate) struct $ty;

        #[doc = concat!("`", $name, "`'s arguments.")]
        #[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
        pub(crate) struct $args {
            $($(#[$meta])* $field: $kind,)*
            /// The task this browsing is for: its log and any OK to ask go there.
            #[serde(default)]
            #[garde(skip)]
            task_id: Option<i64>,
            /// Where: `assistant` (the assistant's own browser), or one of the owner's own Chrome
            /// profiles (their real logins, through the extension) by the name they gave it:
            /// `chrome:work` (`chrome_pair` lists them), or `chrome` for the task's profile.
            /// Omitted: the task's own browser if it has one, else the assistant's. Pick the
            /// profile whose account the task is about; never guess between accounts.
            #[serde(default)]
            #[garde(length(max = 80))]
            browser: Option<String>,
            /// Older form of browser `chrome:<profile>`.
            #[serde(default)]
            #[garde(length(max = 40))]
            chrome: Option<String>,
        }

        impl Tool for $ty {
            type Args = $args;

            fn name(&self) -> &'static str {
                $name
            }

            fn description(&self) -> &'static str {
                $desc
            }

            async fn call(&self, ctx: &ToolContext, $a: $args) -> Result<ToolOutput, ToolError> {
                let who = Who::assistant($a.task_id);
                let browser = $a.browser.clone();
                let chrome = $a.chrome.clone();
                let args = BrowserArgs {
                    browser,
                    chrome,
                    ..$fill
                };
                Ok(output(ctx.app.browser().act($name, &args, &who).await))
            }
        }
    };
}

browser_tool!(
    Open, OpenArgs, "browser_open",
    concat!(intro!(), "Open a URL in the tab in front (new_tab: in a new tab). Returns the page as \
        an accessibility tree with refs ([ref=e12]) for browser_click/browser_type."),
    {
        /// The address.
        #[garde(length(min = 1, max = 4000))]
        url: String,
        /// In a new tab.
        #[serde(default)]
        #[garde(skip)]
        new_tab: Option<bool>,
    },
    |a| BrowserArgs { url: Some(a.url.clone()), new_tab: a.new_tab.unwrap_or(false), ..BrowserArgs::default() }
);

browser_tool!(
    Snapshot, SnapshotArgs, "browser_snapshot",
    "The page in front as an accessibility tree with refs; screenshot: also a picture. A big \
     page comes folded (hidden parts and its biggest menus show as one line with their ref); \
     scope: only part of the page, \"main\" or a ref (a folded menu, a section), to read it whole.",
    {
        /// A picture too.
        #[serde(default)]
        #[garde(skip)]
        screenshot: Option<bool>,
        /// "main" or a ref like e26.
        #[serde(default)]
        #[garde(length(max = 40))]
        scope: Option<String>,
    },
    |a| BrowserArgs { screenshot: a.screenshot.unwrap_or(false), scope: a.scope.clone(), ..BrowserArgs::default() }
);

browser_tool!(
    Click, ClickArgs, "browser_click",
    "Click an element by its ref from the last snapshot. A final step (create, install, allow, \
     submit, send, delete, pay…) needs the owner's OK: on a job they OK'd it goes through; with \
     no OK yet it pauses and asks them with a Go ahead card (or name the job's steps once with \
     browser_plan); once they say Go ahead, click it again. Payments, purchases, passwords, \
     codes and account security ask them on the step every time.",
    {
        /// The element's ref.
        #[serde(rename = "ref")]
        #[garde(length(min = 1, max = 40))]
        at: String,
        /// What it is, in a few words.
        #[serde(default)]
        #[garde(length(max = 120))]
        element: Option<String>,
    },
    |a| BrowserArgs { at: Some(a.at.clone()), element: a.element.clone(), ..BrowserArgs::default() }
);

browser_tool!(
    Type, TypeArgs, "browser_type",
    "Type text into a field by ref (submit: then press Enter). Never a password or a code: those \
     fields are refused; call browser_handoff so the owner types it.",
    {
        /// The field's ref.
        #[serde(rename = "ref")]
        #[garde(length(min = 1, max = 40))]
        at: String,
        /// The text.
        #[garde(length(max = 100_000))]
        text: String,
        /// Then press Enter.
        #[serde(default)]
        #[garde(skip)]
        submit: Option<bool>,
    },
    |a| BrowserArgs { at: Some(a.at.clone()), text: Some(a.text.clone()), submit: a.submit.unwrap_or(false), ..BrowserArgs::default() }
);

browser_tool!(
    Press, PressArgs, "browser_press",
    "Press a key (Enter, Tab, Escape, ArrowDown, Meta+A…). Enter in a form that sends it goes by \
     the same OK as its button.",
    {
        /// The key or chord.
        #[garde(length(min = 1, max = 40))]
        key: String,
    },
    |a| BrowserArgs { key: Some(a.key.clone()), ..BrowserArgs::default() }
);

browser_tool!(
    Autofill,
    AutofillArgs,
    "browser_autofill",
    "Fill the login on this page with the password manager (its extension in this browser, \
     unlocked by the owner): click into the username or password field first. You never see the \
     password; filled fields read ••••••.",
    {},
    |a| BrowserArgs::default()
);

browser_tool!(
    Scroll, ScrollArgs, "browser_scroll",
    "Scroll the page by dy pixels (negative: up).",
    {
        /// Pixels; negative scrolls up.
        #[garde(skip)]
        dy: f64,
    },
    |a| BrowserArgs { dy: Some(a.dy), ..BrowserArgs::default() }
);

browser_tool!(
    WaitFor, WaitForArgs, "browser_wait_for",
    "Wait for text to appear (up to 60 s), or for some seconds.",
    {
        /// The text to wait for.
        #[serde(default)]
        #[garde(length(max = 400))]
        text: Option<String>,
        /// How long.
        #[serde(default)]
        #[garde(skip)]
        seconds: Option<f64>,
    },
    |a| BrowserArgs { text: a.text.clone(), seconds: a.seconds, ..BrowserArgs::default() }
);

/// `browser_tabs`' actions.
#[derive(Clone, Copy, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TabsAction {
    /// List the tabs.
    #[default]
    List,
    /// A new tab (url).
    New,
    /// Bring a tab to the front (index).
    Select,
    /// Close a tab (index).
    Close,
}

impl TabsAction {
    fn word(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::New => "new",
            Self::Select => "select",
            Self::Close => "close",
        }
    }
}

browser_tool!(
    Tabs, TabsArgs, "browser_tabs",
    "Tabs: list, new (url), select (index), close (index).",
    {
        /// What to do.
        #[garde(skip)]
        action: TabsAction,
        /// The tab's index in the list.
        #[serde(default)]
        #[garde(skip)]
        index: Option<i64>,
        /// For new: its address.
        #[serde(default)]
        #[garde(length(max = 4000))]
        url: Option<String>,
    },
    |a| {
        #[expect(clippy::cast_precision_loss, reason = "a tab's index")]
        let index = a.index.map(|i| i as f64);
        BrowserArgs { action: Some(a.action.word().to_owned()), index, url: a.url.clone(), ..BrowserArgs::default() }
    }
);

browser_tool!(
    Close,
    CloseArgs,
    "browser_close",
    "Close the tab in front.",
    {},
    |a| BrowserArgs::default()
);

browser_tool!(
    Handoff, HandoffArgs, "browser_handoff",
    "Hand the browser to the owner: a login, a password, a 2FA code, a CAPTCHA, anything only they \
     should do. Their phone gets a push; you're woken (event \"browser\") when they hand it back. \
     Agents wait meanwhile.",
    {
        /// What they should do, e.g. "Sign in to the store's console".
        #[garde(length(max = 160))]
        reason: String,
    },
    |a| BrowserArgs { reason: Some(a.reason.clone()), ..BrowserArgs::default() }
);

browser_tool!(
    Takeback, TakebackArgs, "browser_takeback",
    "Take the browser back from the owner before they hand it back (after a browser_handoff, or \
     when they took over), when it's needed: they've done what only they could and forgot to hand \
     back, or the task can't wait. It ends their turn at once; they're told in one line and the \
     task logs it. Never while they're still typing a password or code: check browser_snapshot \
     first if unsure.",
    {
        /// Why, in a few words.
        #[garde(length(max = 160))]
        reason: String,
    },
    |a| BrowserArgs { reason: Some(a.reason.clone()), ..BrowserArgs::default() }
);

browser_tool!(
    SaveSecret, SaveSecretArgs, "browser_save_secret",
    "Save an API key or token that a service shows on a page (an OAuth token, an app-level token, \
     a new API key) straight into the server's secrets, without reading it: keys on pages show \
     masked in snapshots as [secret: xoxp-…, use browser_save_secret ref eN]. ref: that ref. \
     name: the secret's file name, as secret_link's. expect_prefix: what it must start with \
     (xoxp-, xapp-…): it refuses otherwise. Returns only its length and first characters. Never \
     for a password (password fields are refused) or on the owner's password and account sites.",
    {
        /// The ref holding the key.
        #[serde(rename = "ref")]
        #[garde(length(min = 1, max = 40))]
        at: String,
        /// Lowercase with dashes, e.g. slack-acme-user.
        #[garde(length(min = 2, max = 64))]
        name: String,
        /// What it must start with, e.g. "xoxp-".
        #[serde(default)]
        #[garde(length(max = 12))]
        expect_prefix: Option<String>,
    },
    |a| BrowserArgs { at: Some(a.at.clone()), name: Some(a.name.clone()), expect_prefix: a.expect_prefix.clone(), ..BrowserArgs::default() }
);

browser_tool!(
    Upload, UploadArgs, "browser_upload",
    "Put a file from the server's machine into a page's file field, with no file dialog. path: \
     the file (/… or ~/…). ref: from browser_snapshot: the file field (or its label), a drop zone \
     holding one, or the button that opens the page's file picker (the picker is caught before \
     it shows). Images, PDFs, documents, zips and app builds up to 50 MB; never anything from \
     the server's secrets, hidden folders or a key file. It only sets the file: the page's Save \
     or Submit is a browser_click afterwards. The task logs the file's name and size.",
    {
        /// The file on the server's machine.
        #[garde(length(min = 1, max = 4000))]
        path: String,
        /// The file field, or the button that opens the picker.
        #[serde(rename = "ref")]
        #[garde(length(min = 1, max = 40))]
        at: String,
    },
    |a| BrowserArgs { path: Some(a.path.clone()), at: Some(a.at.clone()), ..BrowserArgs::default() }
);

browser_tool!(
    Download, DownloadArgs, "browser_download",
    "Save a file a page links to (a PDF, a zip) onto the server's machine. It's fetched inside \
     the tab with that page's own login (no cookie is read), and the tab stays where it is: use \
     this rather than clicking a file link. url: the file's address (or relative to the page), \
     or ref: its link from browser_snapshot. dest: a folder (ending in /) or a file path, in the \
     home or /tmp; never hidden folders or the server's own data; it never overwrites. Up to \
     500 MB. The task logs its name, size and where it went.",
    {
        /// The file's address, or relative to the page.
        #[serde(default)]
        #[garde(length(max = 4000))]
        url: Option<String>,
        /// Its link on the page.
        #[serde(default, rename = "ref")]
        #[garde(length(max = 40))]
        at: Option<String>,
        /// A folder (ending in /) or a file path.
        #[garde(length(min = 1, max = 4000))]
        dest: String,
    },
    |a| BrowserArgs { url: a.url.clone(), at: a.at.clone(), dest: Some(a.dest.clone()), ..BrowserArgs::default() }
);

/// `browser_plan`: a job's final steps named once, up front.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plan;

/// `browser_plan`'s arguments.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct PlanArgs {
    /// What the job does and for whom, e.g. `Set up the test track in the store's console`.
    #[garde(length(min = 3, max = 200))]
    job: String,
    /// The final steps' button labels, e.g. `["Create app", "Install", "Allow"]`.
    #[garde(length(min = 1, max = 12), inner(length(min = 1, max = 60)))]
    steps: Vec<String>,
    /// The task the job is for: the OK is asked and kept there.
    #[garde(skip)]
    task_id: i64,
}

impl Tool for Plan {
    type Args = PlanArgs;

    fn name(&self) -> &'static str {
        "browser_plan"
    }

    fn description(&self) -> &'static str {
        "Before a browser job with final steps (create an app, install, allow, submit, save \
         changes…) on a task with no OK of the owner's yet, name them all at once, so they're \
         asked once, up front, not once per click: job (what it does, for whom), steps (the \
         buttons, as their labels read). One Go ahead / Stop card on the task; their OK covers \
         every step of that job while the task is open. Send, post and publish are decided at \
         the click by their text. Payments, purchases, passwords, codes and account security \
         never ride on an OK: they ask on the step each time."
    }

    async fn call(&self, ctx: &ToolContext, args: PlanArgs) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text(
            ctx.app
                .browser()
                .plan(args.task_id, &args.job, args.steps)
                .await,
        ))
    }
}

/// `chrome_pair`: the owner's paired Chrome profiles, and pairing one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChromePair;

/// `chrome_pair`'s arguments.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema, garde::Validate)]
pub(crate) struct ChromePairArgs {
    /// A profile's name: a new one gets a pairing link; a paired one, a link to re-pair it.
    #[serde(default)]
    #[garde(length(max = 40))]
    profile: Option<String>,
    /// Older form of profile.
    #[serde(default)]
    #[garde(length(max = 40))]
    name: Option<String>,
    /// A new name for that profile.
    #[serde(default)]
    #[garde(length(max = 24))]
    rename: Option<String>,
    /// Cut that profile off.
    #[serde(default)]
    #[garde(skip)]
    unpair: Option<bool>,
}

impl Tool for ChromePair {
    type Args = ChromePairArgs;

    fn name(&self) -> &'static str {
        "chrome_pair"
    }

    fn description(&self) -> &'static str {
        "The owner's Chrome profiles paired with the server (the browser extension in each, each \
         its own Google account). Without profile: the list (name for browser \"chrome:<name>\", \
         account, machine, online, extension version, tasks using it). profile: a new one's name \
         gives them a pairing link to open in that Chrome profile (it walks them through adding \
         the extension, then pairs by itself; 30 minutes, once); a paired one's name gives a link \
         to re-pair it. rename: a new name for that profile. unpair: true cuts it off. They can \
         do all this themselves in the app (Settings, Chrome profiles)."
    }

    async fn call(&self, ctx: &ToolContext, args: ChromePairArgs) -> Result<ToolOutput, ToolError> {
        let profile = args.profile.or(args.name);
        let said = ctx
            .app
            .chrome()
            .pair_tool(
                profile.as_deref(),
                args.unpair.unwrap_or(false),
                args.rename.as_deref(),
            )
            .await
            .map_err(ToolError::failed)?;
        Ok(ToolOutput::text(said))
    }
}
