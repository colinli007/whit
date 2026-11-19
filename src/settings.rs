//! The user's settings, kept as VS Code keeps its own: one `settings.json`
//! with VS Code's setting names, read with comments and trailing commas, and
//! edited one key at a time so that everything else in it stays as written.
//!
//! The file is the only record of the settings: the settings page, the
//! dialogs that remember an answer and an editor on the file all write it,
//! and what the window shows is read back from it. The store is an app-wide
//! global that views observe, as Zed's `SettingsStore` is
//! (`crates/settings/src/settings_store.rs`); this is a rewrite without its
//! registry, layers or migrations.
use std::{collections::BTreeSet, fs, io, path::{Path, PathBuf}};

use gpui_kit::{App, AppContext as _, Global, SharedString};
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstRootNode},
};
use serde_json::{Map, Value};

use crate::{json_file, watcher::DirWatcher};

pub const FILE_NAME: &str = "settings.json";

/// `workbench.activityBar.location`, VS Code 1.117's values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityBarLocation {
    /// Upright, left of the side bar.
    #[default]
    Default,
    /// Across the side bar's title row.
    Top,
    /// Across a row at the bottom of the side bar.
    Bottom,
    Hidden,
}

impl ActivityBarLocation {
    pub const ALL: [Self; 4] = [Self::Default, Self::Top, Self::Bottom, Self::Hidden];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Hidden => "hidden",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|location| location.as_str() == value)
    }
}

/// The settings in effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Values {
    pub activity_bar_location: ActivityBarLocation,
    /// With nothing staged, commit every change without asking.
    pub enable_smart_commit: bool,
    /// With nothing staged, ask whether to commit every change.
    pub suggest_smart_commit: bool,
    /// Ask before pulling and pushing.
    pub confirm_sync: bool,
}

/// VS Code 1.117's defaults.
impl Default for Values {
    fn default() -> Self {
        Self {
            activity_bar_location: ActivityBarLocation::Default,
            enable_smart_commit: false,
            suggest_smart_commit: true,
            confirm_sync: true,
        }
    }
}

/// Where a setting is listed on the settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// 工作台 > 外观
    Appearance,
    /// 扩展 > Git
    Git,
}

impl Category {
    pub const ALL: [Self; 2] = [Self::Appearance, Self::Git];

    /// The table of contents' entry above this one.
    pub fn parent(self) -> &'static str {
        match self {
            Self::Appearance => "工作台",
            Self::Git => "扩展",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Appearance => "外观",
            Self::Git => "Git",
        }
    }
}

/// The control that edits a setting.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Bool,
    /// The values in order, each with what it does.
    Enum(&'static [(&'static str, &'static str)]),
}

/// One setting: how it is listed, and how its value moves between the
/// file and `Values`. Zed's settings page keeps the same pair of functions
/// per item (`crates/settings_ui/src/settings_ui.rs`, `SettingField { pick,
/// write }`); this is a rewrite.
pub struct SettingItem {
    /// VS Code's setting ID, the key in the file.
    pub id: &'static str,
    pub category: Category,
    /// The title's two parts, as the settings page shows `Category: Name`.
    pub title: (&'static str, &'static str),
    /// VS Code's Chinese language pack wording.
    pub description: &'static str,
    pub kind: Kind,
    /// The value in effect, as it is written in the file.
    get: fn(&Values) -> Value,
    /// Takes in a value read from the file; `false` when it is not one this
    /// setting takes, which leaves the default in effect.
    set: fn(&mut Values, &Value) -> bool,
}

impl SettingItem {
    pub fn value(&self, values: &Values) -> Value {
        (self.get)(values)
    }

    pub fn default_value(&self) -> Value {
        (self.get)(&Values::default())
    }

    /// The setting's line as `"id": value`, VS Code's 「将设置复制为 JSON 文本」.
    pub fn json_text(&self, values: &Values) -> String {
        format!("{}: {}", Value::from(self.id), self.value(values))
    }
}

fn bool_value(value: &Value, field: &mut bool) -> bool {
    value.as_bool().map(|value| *field = value).is_some()
}

/// Every setting Whit has, in the order the settings page lists them.
pub static SETTINGS: &[SettingItem] = &[
    SettingItem {
        id: "workbench.activityBar.location",
        category: Category::Appearance,
        title: ("Activity Bar", "Location"),
        description: "控制活动栏相对于主侧边栏的位置。",
        kind: Kind::Enum(&[
            ("default", "在主侧边栏的侧边显示活动栏。"),
            ("top", "在主侧边栏的顶部显示活动栏。"),
            ("bottom", "在主侧边栏的底部显示活动栏。"),
            ("hidden", "隐藏活动栏。"),
        ]),
        get: |values| values.activity_bar_location.as_str().into(),
        set: |values, value| {
            value
                .as_str()
                .and_then(ActivityBarLocation::parse)
                .map(|location| values.activity_bar_location = location)
                .is_some()
        },
    },
    SettingItem {
        id: "git.confirmSync",
        category: Category::Git,
        title: ("Git", "Confirm Sync"),
        description: "同步 Git 存储库前请先进行确认。",
        kind: Kind::Bool,
        get: |values| values.confirm_sync.into(),
        set: |values, value| bool_value(value, &mut values.confirm_sync),
    },
    SettingItem {
        id: "git.enableSmartCommit",
        category: Category::Git,
        title: ("Git", "Enable Smart Commit"),
        description: "在没有暂存的更改时提交所有更改。",
        kind: Kind::Bool,
        get: |values| values.enable_smart_commit.into(),
        set: |values, value| bool_value(value, &mut values.enable_smart_commit),
    },
    SettingItem {
        id: "git.suggestSmartCommit",
        category: Category::Git,
        title: ("Git", "Suggest Smart Commit"),
        description: "建议启用智能提交(在无暂存更改时提交所有更改)。",
        kind: Kind::Bool,
        get: |values| values.suggest_smart_commit.into(),
        set: |values, value| bool_value(value, &mut values.suggest_smart_commit),
    },
];

pub fn item(id: &str) -> Option<&'static SettingItem> {
    SETTINGS.iter().find(|item| item.id == id)
}

/// Why a change was not written.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    /// The file does not parse; VS Code will not write it either.
    Invalid,
    Io(String),
}

pub struct SettingsStore {
    /// `None` when the system has nowhere to keep it.
    file: Option<PathBuf>,
    values: Values,
    /// The known settings the file names, valid or not: VS Code's
    /// 「在当前作用域中配置了」, which shows the modified mark.
    configured: BTreeSet<&'static str>,
    /// The file's text as last read, `None` for no file: the same text
    /// again, such as Whit's own write coming back from the watcher, changes
    /// nothing (Zed's `last_user_settings_content`).
    last_text: Option<Option<String>>,
    /// Why the file did not parse, while it does not.
    error: Option<SharedString>,
}

impl Global for SettingsStore {}

/// VS Code's JSONC: comments and trailing commas, nothing looser.
fn parse_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

/// The settings object in `text`; `None` for an empty file.
fn parse(text: &str) -> Result<Option<Map<String, Value>>, String> {
    jsonc_parser::parse_to_serde_value::<Option<Map<String, Value>>>(text, &parse_options())
        .map_err(|error| error.to_string())
}

/// The file's text, `None` when there is no file.
pub fn read_text(file: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(file) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

impl SettingsStore {
    /// The settings in `~/Library/Application Support/Whit`.
    pub fn load() -> Self {
        Self::load_from(json_file::data_file(FILE_NAME))
    }

    /// The settings kept in `file`, read now: the window's first frame
    /// already follows them. The file is a few hundred bytes.
    pub fn load_from(file: Option<PathBuf>) -> Self {
        let mut store = Self {
            file,
            values: Values::default(),
            configured: BTreeSet::new(),
            last_text: None,
            error: None,
        };
        let text = store.file.as_deref().map_or(Ok(None), read_text);
        match text {
            Ok(text) => {
                store.reload(text);
            }
            Err(error) => log::warn!("cannot read the settings: {error}"),
        }
        store
    }

    /// Follows the file from now on: a change made to it anywhere takes
    /// effect. Its folder is watched, made if missing; the file is not.
    /// Not in unit tests, whose scheduler refuses the watcher thread's
    /// wake-ups: they hand the store the file's text themselves.
    pub fn watch(cx: &mut App) {
        let Some(file) = Self::global(cx).file.clone() else {
            return;
        };
        if let Some(folder) = file.parent()
            && let Err(error) = fs::create_dir_all(folder)
        {
            log::warn!("cannot make {}: {error}", folder.display());
        }
        let real = json_file::real_path(&file);
        let Some(folder) = real.parent() else {
            return;
        };
        let watching = DirWatcher::new().and_then(|(mut watcher, batches)| {
            watcher.watch(folder)?;
            Ok((watcher, batches))
        });
        let (watcher, batches) = match watching {
            Ok(watching) => watching,
            Err(error) => {
                log::error!("cannot watch the settings: {error}");
                return;
            }
        };
        cx.spawn(async move |cx| {
            let _watcher = watcher;
            while let Ok(batch) = batches.recv().await {
                if !batch.rescan && !batch.paths.contains(&real) {
                    continue;
                }
                let read = cx.background_spawn({
                    let file = file.clone();
                    async move { read_text(&file) }
                });
                let text = match read.await {
                    Ok(text) => text,
                    Err(error) => {
                        log::warn!("cannot read the settings: {error}");
                        continue;
                    }
                };
                cx.update_global(|store: &mut Self, _| {
                    store.reload(text);
                });
            }
        })
        .detach();
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    pub fn values(&self) -> &Values {
        &self.values
    }

    /// Whether the file names the setting `id`.
    pub fn is_configured(&self, id: &str) -> bool {
        self.configured.contains(id)
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Takes in the file's text, `None` for no file. A file that does not
    /// parse leaves the settings as they were and records why; a value of
    /// the wrong kind leaves only its own setting at the default. Returns
    /// whether anything was taken in.
    pub fn reload(&mut self, text: Option<String>) -> bool {
        if self.last_text.as_ref() == Some(&text) {
            return false;
        }
        let parsed = text.as_deref().map_or(Ok(None), parse);
        self.last_text = Some(text);
        let object = match parsed {
            Ok(object) => object.unwrap_or_default(),
            Err(error) => {
                log::warn!("ignoring the settings until they parse: {error}");
                self.error = Some(error.into());
                return true;
            }
        };
        self.error = None;
        self.values = Values::default();
        self.configured.clear();
        for item in SETTINGS {
            if let Some(value) = object.get(item.id) {
                self.configured.insert(item.id);
                if !(item.set)(&mut self.values, value) {
                    log::warn!("ignoring {}: {value} is not one of its values", item.id);
                }
            }
        }
        true
    }

    /// Sets `id` to `value` in the file, then takes the new text in at once.
    /// Only that key's text changes; the default value removes the key, as
    /// VS Code does. The file on disk now is edited, not the text last read,
    /// so a change made elsewhere and not seen yet is not lost.
    pub fn update(&mut self, id: &str, value: Value) -> Result<(), WriteError> {
        let Some(file) = self.file.clone() else {
            return Err(WriteError::Io("没有保存设置的位置".into()));
        };
        let io = |error: io::Error| WriteError::Io(error.to_string());
        let text = read_text(&file).map_err(io)?.unwrap_or_default();
        let is_default = item(id).is_some_and(|item| item.default_value() == value);
        let edited = edit(&text, id, (!is_default).then_some(&value))?;
        if edited != text {
            json_file::write_text(&file, &edited).map_err(io)?;
        }
        self.reload(Some(edited));
        Ok(())
    }
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(value) => CstInputValue::Bool(*value),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(value) => CstInputValue::String(value.clone()),
        Value::Array(values) => CstInputValue::Array(values.iter().map(input).collect()),
        Value::Object(object) => CstInputValue::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), input(value)))
                .collect(),
        ),
    }
}

/// `text` with the top-level key `id` set to `value`, or removed for
/// `None`, and nothing else touched. A new file is written as VS Code
/// writes one, indented by four spaces.
fn edit(text: &str, id: &str, value: Option<&Value>) -> Result<String, WriteError> {
    parse(text).map_err(|_| WriteError::Invalid)?;
    if text.trim().is_empty() {
        return Ok(match value {
            Some(value) => format!("{{\n    {}: {value}\n}}\n", Value::from(id)),
            None => text.to_owned(),
        });
    }
    let root = CstRootNode::parse(text, &parse_options()).map_err(|_| WriteError::Invalid)?;
    let object = match root.value() {
        // Only comments: the object goes after them.
        None => root.object_value_or_set(),
        Some(_) => root.object_value().ok_or(WriteError::Invalid)?,
    };
    match (object.get(id), value) {
        (Some(property), Some(value)) => property.set_value(input(value)),
        (Some(property), None) => property.remove(),
        (None, Some(value)) => {
            // An empty object shows no indentation to follow: use VS Code's
            // four spaces, not the parser's guess of two.
            let empty = object.properties().is_empty();
            let property = object.append(id, input(value));
            if empty
                && let Some(indent) = property
                    .previous_sibling()
                    .and_then(|node| node.as_whitespace())
            {
                indent.set_value("    ".into());
            }
        }
        (None, None) => {}
    }
    Ok(root.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn store(directory: &Path) -> SettingsStore {
        SettingsStore::load_from(Some(directory.join(FILE_NAME)))
    }

    fn written(directory: &Path) -> String {
        fs::read_to_string(directory.join(FILE_NAME)).expect("read settings")
    }

    #[test]
    fn defaults_are_vs_code_s() {
        let defaults: Vec<(&str, Value)> = SETTINGS
            .iter()
            .map(|item| (item.id, item.default_value()))
            .collect();
        assert_eq!(
            defaults,
            [
                ("workbench.activityBar.location", Value::from("default")),
                ("git.confirmSync", Value::from(true)),
                ("git.enableSmartCommit", Value::from(false)),
                ("git.suggestSmartCommit", Value::from(true)),
            ]
        );
    }

    #[test]
    fn comments_and_trailing_commas_are_read() {
        let mut store = store(Path::new("/nowhere"));
        store.reload(Some(
            "{\n  // 同步前不确认\n  \"git.confirmSync\": false, /* top */\n  \"workbench.activityBar.location\": \"top\",\n}\n"
                .into(),
        ));
        assert_eq!(store.error(), None);
        assert!(!store.values().confirm_sync);
        assert_eq!(store.values().activity_bar_location, ActivityBarLocation::Top);
    }

    #[test]
    fn a_wrong_value_affects_only_its_setting() {
        let mut store = store(Path::new("/nowhere"));
        store.reload(Some(
            r#"{ "git.confirmSync": "no", "workbench.activityBar.location": "top", "editor.fontSize": 14 }"#
                .into(),
        ));
        assert!(store.values().confirm_sync, "the default stays");
        assert_eq!(store.values().activity_bar_location, ActivityBarLocation::Top);
        // Named in the file, so it shows as modified and can be reset.
        assert!(store.is_configured("git.confirmSync"));
        assert!(!store.is_configured("git.enableSmartCommit"));
    }

    #[test]
    fn a_broken_file_keeps_the_last_settings() {
        let mut store = store(Path::new("/nowhere"));
        store.reload(Some(r#"{ "workbench.activityBar.location": "top" }"#.into()));
        assert!(store.reload(Some(r#"{ "workbench.activityBar.location": "bottom" "#.into())));
        assert!(store.error().is_some());
        assert_eq!(store.values().activity_bar_location, ActivityBarLocation::Top);
        // A missing comma is an error too, as in VS Code.
        assert!(store.reload(Some(r#"{ "a": 1 "b": 2 }"#.into())));
        assert!(store.error().is_some());

        assert!(store.reload(Some(r#"{ "workbench.activityBar.location": "bottom" }"#.into())));
        assert_eq!(store.error(), None);
        assert_eq!(store.values().activity_bar_location, ActivityBarLocation::Bottom);
    }

    #[test]
    fn the_same_text_is_taken_in_once() {
        let mut store = store(Path::new("/nowhere"));
        assert!(store.reload(Some("{}".into())));
        assert!(!store.reload(Some("{}".into())));
        assert!(store.reload(None));
        assert!(!store.reload(None));
    }

    #[test]
    fn no_file_means_the_defaults_and_no_file_is_made() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = store(directory.path());
        assert_eq!(*store.values(), Values::default());
        assert!(!directory.path().join(FILE_NAME).exists());
    }

    #[test]
    fn a_change_edits_only_its_key() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join(FILE_NAME);
        fs::write(&file, "{\n  // 我的设置\n  \"editor.fontSize\": 14\n}\n").unwrap();
        let mut store = store(directory.path());

        store.update("git.confirmSync", false.into()).unwrap();
        assert_eq!(
            written(directory.path()),
            "{\n  // 我的设置\n  \"editor.fontSize\": 14,\n  \"git.confirmSync\": false\n}\n"
        );
        assert!(!store.values().confirm_sync);
        assert!(store.is_configured("git.confirmSync"));

        store.update("git.confirmSync", true.into()).unwrap();
        assert_eq!(
            written(directory.path()),
            "{\n  // 我的设置\n  \"editor.fontSize\": 14\n}\n",
            "the default removes the key"
        );
        assert!(!store.is_configured("git.confirmSync"));
    }

    #[test]
    fn a_value_in_place_is_replaced_in_place() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join(FILE_NAME);
        fs::write(
            &file,
            "{\n    \"workbench.activityBar.location\": \"top\", // 顶部\n    \"x\": 1\n}\n",
        )
        .unwrap();
        let mut store = store(directory.path());
        store
            .update("workbench.activityBar.location", "bottom".into())
            .unwrap();
        assert_eq!(
            written(directory.path()),
            "{\n    \"workbench.activityBar.location\": \"bottom\", // 顶部\n    \"x\": 1\n}\n"
        );
    }

    #[test]
    fn a_new_file_is_indented_by_four_spaces() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut store = store(directory.path());
        store.update("git.enableSmartCommit", true.into()).unwrap();
        assert_eq!(
            written(directory.path()),
            "{\n    \"git.enableSmartCommit\": true\n}\n"
        );
        // Setting a default into no file writes nothing.
        let other = tempfile::tempdir().expect("temporary directory");
        let mut store = SettingsStore::load_from(Some(other.path().join(FILE_NAME)));
        store.update("git.confirmSync", true.into()).unwrap();
        assert!(!other.path().join(FILE_NAME).exists());
    }

    #[test]
    fn edits_on_odd_layouts_still_parse() {
        let cases: &[(&str, &str, Option<Value>)] = &[
            // An old Whit file, indented by two, defaults written out.
            (
                "{\n  \"git.enableSmartCommit\": false,\n  \"git.suggestSmartCommit\": true\n}",
                "git.confirmSync",
                Some(false.into()),
            ),
            (r#"{ "a": 1 }"#, "git.confirmSync", Some(false.into())),
            ("{\n  \"a\": 1,\n}\n", "git.confirmSync", Some(false.into())),
            ("// 只有注释\n", "git.confirmSync", Some(false.into())),
            ("", "git.confirmSync", Some(false.into())),
            ("{\n  \"git.confirmSync\": false\n}\n", "git.confirmSync", None),
            ("{\n  \"a\": 1,\n  \"git.confirmSync\": false\n}\n", "git.confirmSync", None),
        ];
        for (text, id, value) in cases {
            let edited = edit(text, id, value.as_ref()).expect("edited");
            let object = parse(&edited)
                .unwrap_or_else(|error| panic!("{edited:?} does not parse: {error}"))
                .unwrap_or_default();
            assert_eq!(object.get(*id), value.as_ref(), "{text:?} became {edited:?}");
            if text.contains("只有注释") {
                assert!(edited.contains("// 只有注释"), "{edited:?}");
            }
        }
        // The two-space file stays two-space.
        let edited = edit(cases[0].0, "git.confirmSync", Some(&false.into())).unwrap();
        assert!(edited.contains("\n  \"git.confirmSync\": false"), "{edited:?}");
    }

    #[test]
    fn a_broken_file_is_not_written() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let file = directory.path().join(FILE_NAME);
        fs::write(&file, "{ \"a\": ").unwrap();
        let mut store = store(directory.path());
        assert_eq!(
            store.update("git.confirmSync", false.into()),
            Err(WriteError::Invalid)
        );
        assert_eq!(written(directory.path()), "{ \"a\": ");
    }

    #[test]
    fn json_text_is_the_setting_s_line() {
        let item = item("git.confirmSync").unwrap();
        assert_eq!(item.json_text(&Values::default()), "\"git.confirmSync\": true");
    }
}
