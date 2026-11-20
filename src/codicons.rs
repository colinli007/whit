//! VS Code's own icons, the codicons: every icon Whit draws is the one VS
//! Code draws in the same place. The SVGs in `assets/codicons/icons` are
//! copied unmodified from the `@vscode/codicons` package; add one there to use
//! it here.
//!
//! Whit's own views name their codicon with [`Codicon`], picked by place as
//! VS Code picks it: the same Lucide refresh arrows were `refresh` on the
//! source control title and `sync` on the sync button. The component
//! library's views draw Lucide icons by name, so [`for_component_icon`] swaps
//! those for codicons when they load, as a VS Code product icon theme swaps
//! glyphs under the icon ids.
use gpui_kit::{SharedString, assets::IconNamed};

/// The SVGs, served at `codicons/<name>.svg`.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/codicons/icons"]
#[prefix = "codicons/"]
pub(crate) struct CodiconAssets;

/// A codicon, named as VS Code names it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Codicon {
    Add,
    ArrowDown,
    ArrowUp,
    Check,
    ChevronDown,
    ChevronRight,
    Close,
    Cloud,
    CloudUpload,
    CollapseAll,
    DebugDisconnect,
    Discard,
    Ellipsis,
    Files,
    Filter,
    FolderOpened,
    Gear,
    GitBranch,
    GoToFile,
    Refresh,
    Remove,
    /// The settings editor's tab.
    Settings,
    /// The activity bar's Manage.
    SettingsGear,
    SourceControl,
    /// Two panes side by side.
    SplitHorizontal,
    /// Two panes one above the other.
    SplitVertical,
    Sync,
    Tag,
}

impl Codicon {
    fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::ArrowDown => "arrow-down",
            Self::ArrowUp => "arrow-up",
            Self::Check => "check",
            Self::ChevronDown => "chevron-down",
            Self::ChevronRight => "chevron-right",
            Self::Close => "close",
            Self::Cloud => "cloud",
            Self::CloudUpload => "cloud-upload",
            Self::CollapseAll => "collapse-all",
            Self::DebugDisconnect => "debug-disconnect",
            Self::Discard => "discard",
            Self::Ellipsis => "ellipsis",
            Self::Files => "files",
            Self::Filter => "filter",
            Self::FolderOpened => "folder-opened",
            Self::Gear => "gear",
            Self::GitBranch => "git-branch",
            Self::GoToFile => "go-to-file",
            Self::Refresh => "refresh",
            Self::Remove => "remove",
            Self::Settings => "settings",
            Self::SettingsGear => "settings-gear",
            Self::SourceControl => "source-control",
            Self::SplitHorizontal => "split-horizontal",
            Self::SplitVertical => "split-vertical",
            Self::Sync => "sync",
            Self::Tag => "tag",
        }
    }
}

impl IconNamed for Codicon {
    fn path(self) -> SharedString {
        codicon_path(self.name()).into()
    }
}

fn codicon_path(name: &str) -> String {
    format!("codicons/{name}.svg")
}

/// The component library's Lucide icons that show in Whit, by file name,
/// with the codicon VS Code draws for the same thing: menu check marks and
/// submenu arrows, notification severities and close buttons, the diff's
/// unchanged lines, spinners and the editor's find widget.
const COMPONENT_ICONS: &[(&str, &str)] = &[
    ("arrow-down", "arrow-down"),
    ("arrow-up", "arrow-up"),
    ("case-sensitive", "case-sensitive"),
    ("check", "check"),
    ("chevron-down", "chevron-down"),
    ("chevron-left", "chevron-left"),
    ("chevron-right", "chevron-right"),
    ("chevron-up", "chevron-up"),
    ("chevrons-up-down", "unfold"),
    ("circle-check", "pass"),
    ("circle-x", "error"),
    ("close", "close"),
    ("copy", "copy"),
    ("ellipsis", "ellipsis"),
    ("external-link", "link-external"),
    ("info", "info"),
    ("loader", "loading"),
    ("minus", "remove"),
    ("plus", "add"),
    ("replace", "replace"),
    ("search", "search"),
    ("triangle-alert", "warning"),
];

/// The codicon to load in place of the component library's icon at `path`
/// (`icons/<name>.svg`), if VS Code draws one there.
pub(crate) fn for_component_icon(path: &str) -> Option<String> {
    let name = path.strip_prefix("icons/")?.strip_suffix(".svg")?;
    COMPONENT_ICONS
        .iter()
        .find(|(lucide, _)| *lucide == name)
        .map(|(_, codicon)| codicon_path(codicon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn every_codicon_is_bundled() {
        use Codicon::*;
        let named = [
            Add,
            ArrowDown,
            ArrowUp,
            Check,
            ChevronDown,
            ChevronRight,
            Close,
            Cloud,
            CloudUpload,
            CollapseAll,
            DebugDisconnect,
            Discard,
            Ellipsis,
            Files,
            Filter,
            FolderOpened,
            Gear,
            GitBranch,
            GoToFile,
            Refresh,
            Remove,
            Settings,
            SettingsGear,
            SourceControl,
            SplitHorizontal,
            SplitVertical,
            Sync,
            Tag,
        ];
        let paths = named.into_iter().map(|icon| icon.path().to_string()).chain(
            COMPONENT_ICONS
                .iter()
                .map(|(_, codicon)| codicon_path(codicon)),
        );
        for path in paths {
            assert!(CodiconAssets::get(&path).is_some(), "{path} is not bundled");
        }
    }

    #[test]
    fn component_icons_load_as_codicons() {
        assert_eq!(
            for_component_icon("icons/chevrons-up-down.svg").as_deref(),
            Some("codicons/unfold.svg")
        );
        assert_eq!(for_component_icon("icons/battery.svg"), None);
        assert_eq!(for_component_icon("file-icons/rust.svg"), None);
    }
}
