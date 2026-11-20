//! VS Code's own icons, the codicons, for the actions Whit draws as VS Code
//! does. The SVGs in `assets/codicons/icons` are copied unmodified from the
//! `@vscode/codicons` package; add one there to use it here.
use gpui_kit::{SharedString, assets::IconNamed};

/// The SVGs, served at `codicons/<name>.svg`.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/codicons/icons"]
#[prefix = "codicons/"]
pub(crate) struct CodiconAssets;

/// A codicon, named as VS Code names it.
#[derive(Clone, Copy)]
pub(crate) enum Codicon {
    ArrowDown,
    ArrowUp,
    GoToFile,
    /// Two panes side by side.
    SplitHorizontal,
    /// Two panes one above the other.
    SplitVertical,
}

impl IconNamed for Codicon {
    fn path(self) -> SharedString {
        let name = match self {
            Self::ArrowDown => "arrow-down",
            Self::ArrowUp => "arrow-up",
            Self::GoToFile => "go-to-file",
            Self::SplitHorizontal => "split-horizontal",
            Self::SplitVertical => "split-vertical",
        };
        format!("codicons/{name}.svg").into()
    }
}
