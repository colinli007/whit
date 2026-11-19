#!/usr/bin/env python3
"""Generates assets/file-icons from VS Code and its vscode-icons extension, so
the sidebar, tab and breadcrumbs show the icons VS Code shows.

VS Code picks a file icon by, in order: the file name (`fileNames`), the
longest matching extension (`fileExtensions`), the file's language
(`languageIds`), and the default `file` icon. The language comes from the
`languages` VS Code's built-in extensions contribute: a file name match beats
the longest extension match, and a later extension overrides an earlier one.
Other installed extensions and the `files.associations` setting are left out,
so the table is the same on every machine with the same versions.

Needs VS Code in /Applications and vscode-icons installed. Run it again after
updating either:

    python3 scripts/file_icons.py
"""
import json
import re
import shutil
from pathlib import Path

HOME = Path.home()
VSCODE_APP = Path("/Applications/Visual Studio Code.app/Contents/Resources/app")
USER_EXTENSIONS = HOME / ".vscode/extensions"
OUT = Path(__file__).resolve().parent.parent / "assets/file-icons"


def jsonc(text):
    """Parses JSON with comments and trailing commas, as VS Code manifests may be."""
    out, i, in_string = [], 0, False
    while i < len(text):
        c = text[i]
        if in_string:
            out.append(c)
            if c == "\\":
                out.append(text[i + 1])
                i += 1
            elif c == '"':
                in_string = False
        elif c == '"':
            in_string = True
            out.append(c)
        elif text.startswith("//", i):
            i = text.find("\n", i)
            if i < 0:
                break
            continue
        elif text.startswith("/*", i):
            i = text.index("*/", i) + 2
            continue
        else:
            out.append(c)
        i += 1
    return json.loads(re.sub(r",(\s*[}\]])", r"\1", "".join(out)))


def latest(pattern):
    return sorted(USER_EXTENSIONS.glob(pattern))[-1]


def contributed_languages():
    """Each built-in extension's `contributes.languages`."""
    for manifest in sorted((VSCODE_APP / "extensions").glob("*/package.json")):
        try:
            languages = jsonc(manifest.read_text())["contributes"]["languages"]
        except (KeyError, TypeError, ValueError):
            continue
        yield from languages


def main():
    icons_extension = latest("vscode-icons-team.vscode-icons-*")
    theme = json.loads((icons_extension / "dist/src/vsicons-icon-theme.json").read_text())
    definitions = theme["iconDefinitions"]

    def svg(definition):
        return Path(definitions[definition]["iconPath"]).name

    # A few definitions name an SVG the extension does not ship; those files
    # get the default icon instead of none.
    missing = {d for d in definitions if not (icons_extension / "icons" / svg(d)).exists()}
    if missing:
        print(f"skipped {len(missing)} definitions without an SVG: {sorted(missing)}")

    language_names, language_extensions = {}, {}
    for language in contributed_languages():
        for name in language.get("filenames", []):
            language_names[name.lower()] = language["id"]
        for extension in language.get("extensions", []):
            language_extensions[extension.lower().lstrip(".")] = language["id"]
        for pattern in language.get("filenamePatterns", []):
            # Only `*.suffix` patterns; other globs are rare and need a matcher.
            if re.fullmatch(r"\*\.[^*?/\[\]{}]+", pattern):
                language_extensions[pattern[2:].lower()] = language["id"]

    def present(mapping):
        return {key: svg(d) for key, d in mapping.items() if d not in missing}

    languages = present(theme["languageIds"])
    table = {
        "file": svg(theme["file"]),
        # The explorer's folders: the default pair only, closed and open.
        "folder": svg(theme["folder"]),
        "folderExpanded": svg(theme["folderExpanded"]),
        "fileNames": {name.lower(): icon for name, icon in present(theme["fileNames"]).items()},
        "fileExtensions": {ext.lower(): icon for ext, icon in present(theme["fileExtensions"]).items()},
        # Languages without an icon would fall back to the default anyway.
        "languageFileNames": {n: l for n, l in language_names.items() if l in languages},
        "languageExtensions": {e: l for e, l in language_extensions.items() if l in languages},
        "languageIds": languages,
    }

    if OUT.exists():
        shutil.rmtree(OUT)
    (OUT / "icons").mkdir(parents=True)
    used = {table["file"], table["folder"], table["folderExpanded"]}
    used |= {*table["fileNames"].values(), *table["fileExtensions"].values()}
    used |= set(languages.values())
    for name in sorted(used):
        shutil.copy(icons_extension / "icons" / name, OUT / "icons" / name)
    (OUT / "theme.json").write_text(json.dumps(table, indent=1, sort_keys=True) + "\n")
    shutil.copy(icons_extension / "LICENSE.txt", OUT / "LICENSE-vscode-icons.txt")
    version = json.loads((icons_extension / "package.json").read_text())["version"]
    (OUT / "README.md").write_text(
        f"File icons of [vscode-icons](https://github.com/vscode-icons/vscode-icons) {version},\n"
        "generated by `scripts/file_icons.py`. The icons are licensed under\n"
        "[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/), the extension's code\n"
        "under the MIT license (LICENSE-vscode-icons.txt); branded icons remain under\n"
        "their owners' copyright.\n"
    )
    print(f"{len(used)} icons, {len(table['languageExtensions'])} language extensions -> {OUT}")


if __name__ == "__main__":
    main()
