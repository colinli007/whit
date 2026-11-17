//! Whit: VS Code's Source Control view as a standalone app. The binary
//! only opens the window; everything else lives here, so the off-screen
//! screenshot tests in `tests/visual.rs` can drive the same window.
mod file_icons;
mod git;
mod json_file;
mod recent;
mod settings;
mod terminal;
mod theme;
mod workbench;

use std::borrow::Cow;

use gpui_kit::*;
pub use recent::RecentFolders;
pub use settings::Settings;
pub use workbench::Workbench;

actions!(
    whit,
    [
        About,
        OpenFolder,
        CloseFolder,
        Refresh,
        OpenTerminal,
        Quit,
        Commit,
        CommitAmend,
        CommitAndPush,
        CommitAndSync,
        Pull,
        Push,
        Sync,
        Fetch,
        Checkout,
        CreateBranch,
        CreateBranchFrom,
        RenameBranch,
        DeleteBranch,
        DeleteRemoteBranch,
        MergeBranch,
        RebaseBranch,
        PublishBranch,
        AbortMerge,
        AbortRebase,
        StageAll,
        UnstageAll,
        DiscardAll,
    ]
);

pub const APP_NAME: &str = "Whit";

// Icons beyond the component library's default set.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Cloud,
        CloudUpload,
        Columns2,
        FileSymlink,
        GitBranch,
        RefreshCw,
        Rows2,
        Tag,
        Undo2,
        Unplug,
        X
    ]
);

/// The component library's icons, the extra ones above and the file icons.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = file_icons::FileIconAssets::get(path) {
            return Ok(Some(icon.data));
        }
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Registers what the window relies on. UI tests call it too, so they drive
/// the same key bindings and actions as the menus and shortcuts.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    theme::init(cx);
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenFolder, None),
        KeyBinding::new("cmd-r", Refresh, None),
        KeyBinding::new("cmd-j", OpenTerminal, None),
        KeyBinding::new("cmd-q", Quit, None),
        // Registered after the text field's own bindings, so in the commit
        // message it wins over the field's cmd-enter, which inserts a newline.
        KeyBinding::new("cmd-enter", Commit, Some(workbench::COMMIT_MESSAGE_CONTEXT)),
    ]);
    cx.set_menus(vec![
        Menu::new(APP_NAME).items([
            MenuItem::action("关于 Whit", About),
            MenuItem::separator(),
            MenuItem::action("退出", Quit),
        ]),
        Menu::new("文件").items([
            MenuItem::action("打开文件夹…", OpenFolder),
            MenuItem::action("关闭文件夹", CloseFolder),
            MenuItem::separator(),
            MenuItem::action("刷新", Refresh),
            MenuItem::action("在终端中打开", OpenTerminal),
        ]),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}
