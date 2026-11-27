//! The Mac tools (`mac_apps`, `mac_app_*`, `mac_screen`): the owner's Mac through its helper,
//! one app at a time in the background, or the whole screen. Their logic is the hands
//! service's ([`crate::services::hands`]).

use base64::Engine as _;
use emberhut_api::mcp::Content;
use serde::Deserialize;

use super::{Tool, ToolContext, ToolError, ToolOutput};
use crate::services::browser::Act;
use crate::services::browser::surface::Who;
use crate::services::hands::HandsArgs;

/// What every app tool's description starts with.
macro_rules! intro {
    () => {
        "The owner's Mac, one app at a time in the background (through the Mac helper): their \
         mouse doesn't move and the app they're in stays in front. "
    };
}

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

/// One Mac tool: its type, its arguments (with `task_id` added), its name and description, and
/// how its arguments become the service's.
macro_rules! mac_tool {
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
            /// The task this is for: its log and any OK to ask go there.
            #[serde(default)]
            #[garde(skip)]
            task_id: Option<i64>,
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
                let args: HandsArgs = $fill;
                Ok(output(ctx.app.hands().act($name, &args, &who).await))
            }
        }
    };
}

mac_tool!(
    Apps,
    AppsArgs,
    "mac_apps",
    concat!(
        intro!(),
        "The apps running on the Mac and their windows. Touches nothing."
    ),
    {},
    |a| HandsArgs::default()
);

mac_tool!(
    AppOpen, AppOpenArgs, "mac_app_open",
    concat!(intro!(), "Open an app without bringing it forward (hidden, unless visible). Finder, \
        Xcode, Simulator and Terminal (to look at only) never ask; on a job the owner OK'd no app \
        asks; else an app is asked once (\"Let the assistant use Pages on your Mac?\") and \
        remembered: after their Go ahead, call again."),
    {
        /// The app by its name (Pages, Notes, Calculator), bundle id or pid.
        #[garde(length(min = 1, max = 200))]
        app: String,
        /// Bring it forward.
        #[serde(default)]
        #[garde(skip)]
        visible: Option<bool>,
    },
    |a| HandsArgs { app: Some(a.app.clone()), visible: a.visible, ..HandsArgs::default() }
);

mac_tool!(
    AppSnapshot, AppSnapshotArgs, "mac_app_snapshot",
    concat!(intro!(), "One window of an app as a list of elements with refs ([a12] button \"Save\") \
        and its menus; picture: also a picture of that window only. window: which one, by number \
        or title (default its main one). scope: a ref, to read only that part. Refs last until the \
        next full snapshot of that app."),
    {
        /// The app.
        #[garde(length(min = 1, max = 200))]
        app: String,
        /// Which window.
        #[serde(default)]
        #[garde(length(max = 200))]
        window: Option<String>,
        /// A ref, to read only that part.
        #[serde(default)]
        #[garde(length(max = 40))]
        scope: Option<String>,
        /// A picture of the window too.
        #[serde(default)]
        #[garde(skip)]
        picture: Option<bool>,
        /// The most elements to list.
        #[serde(default)]
        #[garde(skip)]
        limit: Option<i64>,
    },
    |a| HandsArgs {
        app: Some(a.app.clone()),
        window: a.window.clone(),
        scope: a.scope.clone(),
        picture: a.picture,
        limit: a.limit,
        ..HandsArgs::default()
    }
);

mac_tool!(
    AppClick, AppClickArgs, "mac_app_click",
    concat!(intro!(), "Press an element by its ref (or its exact label, when only one has it). \
        action: another of its actions (ShowMenu, Increment…). A final step goes through on a job \
        the owner OK'd; payments, passwords, codes and account security ask them each time, and so \
        does one with no OK on the job yet; once they say Go ahead, call it again. Take a snapshot \
        after to see what happened."),
    {
        /// The app.
        #[garde(length(min = 1, max = 200))]
        app: String,
        /// The element's ref.
        #[serde(rename = "ref")]
        #[garde(length(min = 1, max = 200))]
        at: String,
        /// What it is, in a few words.
        #[serde(default)]
        #[garde(length(max = 120))]
        element: Option<String>,
        /// Another of its actions.
        #[serde(default)]
        #[garde(length(max = 60))]
        action: Option<String>,
    },
    |a| HandsArgs {
        app: Some(a.app.clone()),
        at: Some(a.at.clone()),
        element: a.element.clone(),
        action: a.action.clone(),
        ..HandsArgs::default()
    }
);

mac_tool!(
    AppType, AppTypeArgs, "mac_app_type",
    concat!(intro!(), "Type into an app. ref: a field, filled directly (or typed into); without \
        ref, typed into whatever is focused in that app. key: then press a key (Return, Tab, \
        Escape, Up, cmd+s, shift+tab…). Never a password or a code. Return in a messaging app \
        (Messages, Mail, Slack, WhatsApp) sends: it goes by the owner's OK."),
    {
        /// The app.
        #[garde(length(min = 1, max = 200))]
        app: String,
        /// The text.
        #[serde(default)]
        #[garde(length(max = 100_000))]
        text: Option<String>,
        /// A field's ref.
        #[serde(default, rename = "ref")]
        #[garde(length(max = 200))]
        at: Option<String>,
        /// Then press this key.
        #[serde(default)]
        #[garde(length(max = 40))]
        key: Option<String>,
    },
    |a| HandsArgs {
        app: Some(a.app.clone()),
        text: a.text.clone(),
        at: a.at.clone(),
        key: a.key.clone(),
        ..HandsArgs::default()
    }
);

mac_tool!(
    AppMenu, AppMenuArgs, "mac_app_menu",
    concat!(intro!(), "An app's menu bar: path \"File\" lists that menu; \"File > Export…\" picks \
        the item (\"Format > Font > Bold\" goes deeper). A final step asks the owner first."),
    {
        /// The app.
        #[garde(length(min = 1, max = 200))]
        app: String,
        /// The menu path.
        #[garde(length(min = 1, max = 400))]
        path: String,
    },
    |a| HandsArgs { app: Some(a.app.clone()), path: Some(a.path.clone()), ..HandsArgs::default() }
);

/// `mac_screen`'s actions.
#[derive(Clone, Copy, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ScreenAction {
    /// Start driving (reason, minutes).
    #[default]
    Start,
    /// The screen as a picture.
    Shot,
    /// Click at x, y.
    Click,
    /// Double-click at x, y.
    DoubleClick,
    /// Right-click at x, y.
    RightClick,
    /// Move the mouse to x, y.
    Move,
    /// Scroll by dy.
    Scroll,
    /// Type text.
    Type,
    /// Press a key.
    Key,
    /// Stop driving.
    Stop,
}

impl ScreenAction {
    fn word(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Shot => "shot",
            Self::Click => "click",
            Self::DoubleClick => "double_click",
            Self::RightClick => "right_click",
            Self::Move => "move",
            Self::Scroll => "scroll",
            Self::Type => "type",
            Self::Key => "key",
            Self::Stop => "stop",
        }
    }
}

mac_tool!(
    Screen, ScreenArgs, "mac_screen",
    "The owner's whole Mac with the real mouse and keyboard, only when the mac_app tools can't do \
     it (an app with no elements, a drag, something across apps). start (reason, minutes ≤ 60): \
     no ask on a job the owner OK'd, or when the Mac has been idle 5+ minutes; else it asks them \
     once (their Go ahead covers the job: call start again). A bar on the screen says the \
     assistant is driving, with Stop, and their touch on the mouse or keyboard stops it: then \
     don't start again on your own. shot: the screen as a picture, x,y in its pixels. click, \
     double_click, right_click, move (x, y), scroll (dy, x,y optional), type (text), key \
     (Return, cmd+c…). stop when done. element: what you click, in a few words (a final step goes \
     by the job's OK).",
    {
        /// What to do.
        #[garde(skip)]
        action: ScreenAction,
        /// x, in the screen's pixels.
        #[serde(default)]
        #[garde(skip)]
        x: Option<f64>,
        /// y, in the screen's pixels.
        #[serde(default)]
        #[garde(skip)]
        y: Option<f64>,
        /// Text to type.
        #[serde(default)]
        #[garde(length(max = 100_000))]
        text: Option<String>,
        /// A key to press.
        #[serde(default)]
        #[garde(length(max = 40))]
        key: Option<String>,
        /// Pixels to scroll.
        #[serde(default)]
        #[garde(skip)]
        dy: Option<f64>,
        /// What you click, in a few words.
        #[serde(default)]
        #[garde(length(max = 120))]
        element: Option<String>,
        /// What you'll do, in a few words (start).
        #[serde(default)]
        #[garde(length(max = 200))]
        reason: Option<String>,
        /// For how long (start).
        #[serde(default)]
        #[garde(skip)]
        minutes: Option<f64>,
    },
    |a| HandsArgs {
        action: Some(a.action.word().to_owned()),
        x: a.x,
        y: a.y,
        text: a.text.clone(),
        key: a.key.clone(),
        dy: a.dy,
        element: a.element.clone(),
        reason: a.reason.clone(),
        minutes: a.minutes,
        ..HandsArgs::default()
    }
);
