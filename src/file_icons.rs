//! File icons as VS Code shows them with the vscode-icons theme, from the
//! table and SVGs `scripts/file_icons.py` puts in `assets/file-icons`.
use std::{collections::HashMap, sync::LazyLock};

use gpui_kit::SharedString;
use serde::Deserialize;

/// The icons' SVGs, served at `file-icons/<name>.svg`.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/file-icons/icons"]
#[prefix = "file-icons/"]
pub(crate) struct FileIconAssets;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IconTable {
    file: String,
    folder: String,
    folder_expanded: String,
    file_names: HashMap<String, String>,
    file_extensions: HashMap<String, String>,
    language_file_names: HashMap<String, String>,
    language_extensions: HashMap<String, String>,
    language_ids: HashMap<String, String>,
}

static TABLE: LazyLock<IconTable> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/file-icons/theme.json"))
        .expect("the bundled icon table parses")
});

/// The asset path of the icon for the file at `path`. VS Code tries the file
/// name, then its longest extension, then its language, then the default.
pub fn icon_for(path: &str) -> SharedString {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    let table = &*TABLE;
    let icon = table
        .file_names
        .get(&name)
        .or_else(|| longest_extension(&name, &table.file_extensions))
        .or_else(|| {
            let language = table
                .language_file_names
                .get(&name)
                .or_else(|| longest_extension(&name, &table.language_extensions))?;
            table.language_ids.get(language)
        })
        .unwrap_or(&table.file);
    format!("file-icons/{icon}").into()
}

/// The asset path of the explorer's folder icon, closed or open.
pub fn folder_icon(expanded: bool) -> SharedString {
    let table = &*TABLE;
    let icon = if expanded {
        &table.folder_expanded
    } else {
        &table.folder
    };
    format!("file-icons/{icon}").into()
}

/// The entry for the longest of `name`'s extensions: `test.ts` before `ts`.
/// A leading dot starts one too, as `.gitignore` has the extension `gitignore`.
fn longest_extension<'a>(name: &str, table: &'a HashMap<String, String>) -> Option<&'a String> {
    name.match_indices('.')
        .find_map(|(ix, _)| table.get(&name[ix + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_follow_vs_code_precedence() {
        // By language: `.ts` and `.tsx` are only languages' extensions.
        assert_eq!(
            icon_for("src/model.ts"),
            "file-icons/file_type_typescript.svg"
        );
        assert_eq!(
            icon_for("src/Layer.tsx"),
            "file-icons/file_type_reactts.svg"
        );
        // A longer extension wins.
        assert_eq!(
            icon_for("a/types.d.ts"),
            "file-icons/file_type_typescriptdef.svg"
        );
        // A file name beats its extension, whatever its case.
        assert_eq!(icon_for("web/Package.JSON"), "file-icons/file_type_npm.svg");
        assert_eq!(icon_for(".gitignore"), "file-icons/file_type_git.svg");
        // Nothing matches.
        assert_eq!(icon_for("notes.unknownext"), "file-icons/default_file.svg");
        assert_eq!(folder_icon(false), "file-icons/default_folder.svg");
        assert_eq!(folder_icon(true), "file-icons/default_folder_opened.svg");
    }

    #[test]
    fn every_icon_in_the_table_is_bundled() {
        let table = &*TABLE;
        let icons = [&table.file, &table.folder, &table.folder_expanded]
            .into_iter()
            .chain(table.file_names.values())
            .chain(table.file_extensions.values())
            .chain(table.language_ids.values());
        for icon in icons {
            assert!(
                FileIconAssets::get(&format!("file-icons/{icon}")).is_some(),
                "{icon} is bundled"
            );
        }
    }
}
