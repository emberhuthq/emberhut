//! The files built into the app: its face picture and fonts, then the icons it draws:
//! gpui-kit's default bundle (the components' own icons) and the Lucide icons the app's views
//! name beyond it ([`AppIcons`]).

use std::borrow::Cow;

use gpui_kit::{App, AssetSource, Result, SharedString};

/// The assistant's face, as the sidebar and the sign-in screen show it.
pub(crate) const FACE: &str = "face.png";

/// The interface font: the Mac's own system font, and Inter (built in) elsewhere so the app
/// looks the same.
#[cfg(target_os = "macos")]
pub(crate) const UI_FONT: &str = ".SystemUIFont";
/// The interface font: the Mac's own system font, and Inter (built in) elsewhere so the app
/// looks the same.
#[cfg(not(target_os = "macos"))]
pub(crate) const UI_FONT: &str = "Inter";

// The Lucide icons the views name that gpui-kit's default bundle lacks: without them an icon
// draws blank. Only these are built in, not the whole catalog (about 1 MiB more); the tests
// below fail when a view names an icon missing here, or when one here is no longer used.
gpui_kit::assets::icon_assets!(
    AppIcons,
    [
        AlarmClock,
        Archive,
        ArchiveRestore,
        ArrowLeftRight,
        BadgeCheck,
        Banknote,
        Brain,
        Briefcase,
        CalendarClock,
        ChartBar,
        ChartColumn,
        Circle,
        CircleArrowDown,
        CircleArrowUp,
        CircleDashed,
        CircleDot,
        CircleMinus,
        CirclePause,
        CirclePlay,
        CirclePlus,
        CircleQuestionMark,
        CircleStop,
        Clipboard,
        Clock,
        CloudOff,
        Cog,
        Contact,
        CornerDownRight,
        Disc,
        Download,
        FileArchive,
        FileChartColumn,
        FileSearch,
        FileSpreadsheet,
        Film,
        Flag,
        FlagTriangleRight,
        FolderSearch,
        Footprints,
        GitBranch,
        GitPullRequest,
        Hand,
        Hash,
        House,
        Image,
        Images,
        KeyRound,
        Layers,
        Link,
        ListChecks,
        Lock,
        MessageCircle,
        MessageSquareText,
        MessagesSquare,
        MoonStar,
        Music,
        Paperclip,
        Pencil,
        Presentation,
        Radio,
        Repeat,
        Reply,
        Share,
        Share2,
        Sheet,
        SlidersHorizontal,
        SquarePen,
        Terminal,
        Ticket,
        Trash,
        UserRound,
        Users,
        Video,
        WandSparkles,
        WifiOff,
        Wrench,
        X,
        ZoomIn,
        ZoomOut,
    ]
);

/// The app's assets.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == FACE {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/face.png"))));
        }
        if let Some(icon) = AppIcons.load(path)? {
            return Ok(Some(icon));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Registers the built-in fonts (Inter, under the SIL Open Font License: see
/// `assets/fonts/Inter-LICENSE.txt`).
pub(crate) fn register_fonts(cx: &mut App) {
    let fonts = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf").as_slice()),
    ];
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        log::warn!("couldn't load the built-in fonts ({error}); using the system's");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use gpui_kit::assets::IconName;

    use super::*;

    /// The names a view gives an asset icon: `IconName` is gpui-kit's (its components' own
    /// enum names the same icons), `AssetIcon` the name some views import it as.
    const NAMES: [&str; 2] = ["IconName::", "AssetIcon::"];

    /// Every icon variant the app's source names, as `IconName::X` or `AssetIcon::X`.
    fn named_in(dir: &Path, found: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                named_in(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for name in NAMES {
                    for (at, _) in text.match_indices(name) {
                        let before = text[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let variant: String = text[at + name.len()..]
                            .chars()
                            .take_while(char::is_ascii_alphanumeric)
                            .collect();
                        if variant.starts_with(|c: char| c.is_ascii_uppercase()) {
                            found.insert(variant);
                        }
                    }
                }
            }
        }
    }

    /// The icons the app's source names.
    fn used() -> Vec<IconName> {
        let mut found = BTreeSet::new();
        named_in(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut found,
        );
        let icons: Vec<IconName> = IconName::ALL
            .iter()
            .copied()
            .filter(|icon| found.contains(&format!("{icon:?}")))
            .collect();
        assert!(
            icons.len() > 100,
            "the scan found only {} icons",
            icons.len()
        );
        icons
    }

    #[test]
    fn every_icon_the_views_name_loads() {
        let blank: Vec<String> = used()
            .into_iter()
            .filter(|icon| !matches!(Assets.load(&icon.path()), Ok(Some(_))))
            .map(|icon| format!("{icon:?}"))
            .collect();
        assert!(
            blank.is_empty(),
            "these icons would draw blank: add them to AppIcons: {blank:?}"
        );
    }

    #[test]
    fn the_extra_icons_are_used_and_not_in_the_default_bundle() {
        let used: BTreeSet<String> = used().into_iter().map(|icon| icon.path().into()).collect();
        for path in AppIcons.list("").unwrap() {
            assert!(used.contains(path.as_ref()), "{path} is no longer used");
            assert!(
                gpui_kit::assets::Assets.load(&path).is_err(),
                "{path} is in the default bundle already"
            );
        }
    }

    #[test]
    fn the_list_has_both_sources_once() {
        let listed = Assets.list("icons/").unwrap();
        assert!(
            listed
                .iter()
                .any(|path| path.as_ref() == "icons/paperclip.svg")
        );
        assert!(listed.iter().any(|path| path.as_ref() == "icons/check.svg"));
        assert!(listed.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
