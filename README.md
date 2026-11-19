# Whit

English | [简体中文](README.zh-CN.md)

A git client for reviewing and committing changes, with the layout and interactions of VS Code's Source Control view (VS Code 1.117). The UI and code colors are VS Code's Monokai Dimmed, code is set in Fira Code 14px (with ligatures; falls back to the system monospace font when it is not installed), and file icons come from vscode-icons (see "File icons" below). The UI text is in Simplified Chinese, matching VS Code with the Chinese language pack; the names below are those of VS Code in English.

- **Welcome page**: shown when no folder is open; "Recent" lists recently opened repositories (up to 50 are kept, 10 shown by default).
- **Source Control sidebar**: commit message (⌘Enter commits). The action button below the input changes with the repository state as in VS Code: with changes it is "Commit" with a dropdown (Commit / Commit (Amend) / Commit & Push / Commit & Sync); without changes it is "Sync Changes N↓ M↑" or "Publish Branch"; when a merge or rebase stops on conflicts it is "Continue". Files are grouped into "Merge Changes", "Staged Changes" and "Changes"; hovering a file row offers Open, Stage, Unstage and Discard Changes.
- **Editor**: clicking a file shows its full side-by-side diff (unchanged lines are not folded). Long lines are clipped within each column and both columns scroll horizontally together. Switch between side-by-side and inline, and jump to the previous / next change. Indent guides and bracket pair guides are drawn as in VS Code (click in the code to highlight the current indent block and bracket pair), and the overview ruler on the right shows changes (deletions in the left lane, additions in the right; the gray slider is the visible range and can be clicked, dragged and scrolled).
- **Status bar**: branch name (with `*` when there are changes), click for "Checkout to"; `↓ ↑` incoming / outgoing commit counts, click to sync; "Publish Branch" when the branch has no upstream; no sync entry on a detached HEAD.
- **"…" menu** (structured like VS Code's): Pull, Push, Checkout to..., Fetch, and the "Commit", "Changes", "Pull, Push" and "Branch" submenus.
- **Branches**: a VS Code-style Quick Pick (centered at the top of the window, type to filter). Check out branches / remote branches / tags (a remote branch gets a local tracking branch); when local changes block a checkout, choose Stash & Checkout / Migrate Changes / Force Checkout. Create a branch, create one from a chosen ref, check out detached; rename and delete local / remote branches; merge and rebase, click "Continue" once conflicts are resolved and staged, or abort the merge / rebase; publish a branch (choosing the remote when there are several).
- **⌘J Open in Terminal**: opens a new Ghostty window or tab in the repository root (also in the "File" menu); requires Ghostty.
- **Settings** (⌘, or Whit → Preferences → Settings, also with no folder open): VS Code's Settings editor in a tab, with search (including `@modified`), the "User" scope and a `{}` button that opens `settings.json`. It lists only the settings Whit has: `workbench.activityBar.location` (`default`, `top`, `bottom`, `hidden`; ⇧⌘E and ⌃⇧G switch views without the activity bar), `git.confirmSync`, `git.enableSmartCommit` and `git.suggestSmartCommit`.

The view refreshes when the window regains focus, on cmd-R, and after every write operation.

## Notes on write operations

- Staging, committing, pulling and pushing call your local `git`, follow your git configuration, and run the repository's hooks (pre-commit and so on) and commit signing, exactly as in a terminal.
- Opening and browsing a repository never runs external programs from its configuration (fsmonitor, external diff and textconv are turned off for reads).
- Remote authentication uses only the credentials already set up on your machine (ssh-agent, Keychain, credential helpers). An operation that would need a password or passphrase fails with a message; set up authentication in a terminal and retry.
- Discarding an untracked file deletes it permanently, after a confirmation.
- "Force Checkout" throws away all uncommitted changes and deletes untracked files; deleting a branch that is not fully merged asks once more.
- The recent projects list is stored in `~/Library/Application Support/Whit/recent.json`.
- Settings are stored in `~/Library/Application Support/Whit/settings.json` under VS Code's setting names. It is read as VS Code reads its own (comments and trailing commas allowed), and a change edits only its own key, keeping everything else in the file as written; a value set back to its default is removed. Edits made to the file elsewhere apply within a second; while it does not parse, the last settings that did stay in effect.

## Download

Download the `.dmg` from [Releases](https://github.com/colinli007/whit/releases/latest): `aarch64` for Apple silicon, `x64` for Intel. Requires macOS 12 or later and `git`.

Open the `.dmg` and drag Whit into Applications. The app is not notarized by Apple, so the first launch is blocked with "Apple could not verify…". Either:

- click "Open Anyway" at the bottom of System Settings → Privacy & Security, or
- run `xattr -dr com.apple.quarantine /Applications/Whit.app` in a terminal, then open it as usual.

To release a new version, bump `version` in `Cargo.toml`, commit, and push a matching tag (for example `git tag v0.2.0 && git push origin v0.2.0`). GitHub Actions builds the `.dmg` for both architectures and attaches them to that tag's release.

## Performance

Idle memory after opening the same frontend project (1813 tracked files) with each app's out-of-the-box defaults and doing nothing else (Activity Monitor's "Memory", summed over the whole process tree, median of 12 samples):

| App | Memory | Processes |
|---|---|---|
| **Whit 0.1.0** | **79.4 MB** | 1 |
| Zed 1.28.1 | 171.9 MB | 2 |
| VS Code 1.117.0 | 583.3 MB | 9 |

See [docs/performance.md](docs/performance.md) for the setup, the method and a per-process breakdown; `scripts/memory_bench.sh` reproduces it.

## Building

- Rust (stable, edition 2024)
- On macOS, Xcode's Metal Toolchain (Xcode → Settings → Components), which gpui needs to compile its shaders
- `git` installed at runtime

## Running

```sh
cargo run --release -- <repository path>
```

Started without a path, it shows the welcome page; open a folder with "Open…", cmd-O or a recent project.

## Tests

- `cargo test`: unit tests for git reads and writes, and headless UI tests that drive the whole window on gpui's test platform (clicks, keystrokes and confirmation dialogs are simulated; no window is shown).
- `cargo test --features visual-tests --test visual`: screenshot tests. They use the real macOS platform and Metal renderer with the window off screen at (-10000, -10000) and never focused; events are injected into the app and screenshots are read back from the renderer into `target/visual-shots/`. They never move the mouse or steal focus, and run while the screen is locked. Native confirmation dialogs are replaced with in-window ones (button ids `prompt-<label>`) so they can be captured and clicked. AppKit must run on the main thread, so this test target has its own `main` (`harness = false`); scenarios are registered in `main` in `tests/visual.rs`.

## File icons

The file icons in the sidebar rows, editor tabs and breadcrumbs come from [vscode-icons](https://github.com/vscode-icons/vscode-icons), live in `assets/file-icons/` and are embedded at compile time. Icons are picked by VS Code's rules: file name → longest extension → the file's language → default icon.

`assets/file-icons/` is generated by `python3 scripts/file_icons.py`, which reads VS Code and the installed vscode-icons extension from `/Applications`: the extension's icon theme, and the languages declared by VS Code's built-in extensions (which extensions and file names belong to which language). No other extensions or user settings are read, so the same versions of VS Code and vscode-icons produce the same result on any machine. Run it again after either updates. The icons are licensed under CC BY-SA 4.0 and the extension code under MIT (see `assets/file-icons/README.md`); brand icons remain the property of their owners.

## Bundled gpui-component patches

`vendor/gpui-component` is a copy of gpui-component 0.9.1 that replaces the upstream crate through `[patch.crates-io]` in `Cargo.toml`, licensed under Apache 2.0 (`vendor/gpui-component/LICENSE-APACHE`). There are eight changes:

- Without wrapping, the diff component splits the width evenly between the two columns, clips code within each column, and scrolls all rows of both columns with one shared horizontal offset (like VS Code's side-by-side diff); upstream sizes each column to its longest line, so long lines push the right column out of the window.
- New `Diff::change_colors` sets the backgrounds of added / removed lines and words directly (to match VS Code's `diffEditor.*` colors); upstream can only multiply theme colors by a fixed opacity.
- Syntax highlighting no longer emits the `local.*` captures of the locals query: upstream disabled only the injections section, and the locals captures override real highlight names such as `variable.parameter`, leaving parameters and the like uncolored in JS/TS/TSX/Ruby/Scala/Lua/Svelte.
- The command palette `Command` gains `search_icon(bool)` (on by default); the Quick Pick uses it to drop the magnifier in front of the search box, as in VS Code.
- TSX uses the full TypeScript highlight query plus JSX rules, and JavaScript gains the JSX rules too: upstream TSX used tree-sitter-typescript's `HIGHLIGHTS_QUERY` on its own, which is only the few dozen lines TypeScript adds on top of JavaScript (meant to be layered over the JS rules), so keywords, strings, function names, numbers and JSX tags in `.tsx` files went uncolored.
- Make the diff look like VS Code's:
  - `Diff::line_height_ratio` (line height = font size × ratio, rounded); the code font size is the theme's `mono_font.size` as is (as in the component's code editor), where upstream also multiplied it by rem/16. Line numbers use the theme's `editor.line_number` color.
  - `Diff::guides(DiffGuideColors)`: indent guides and bracket pair guides drawn with VS Code's algorithm (`src/diff/guides.rs`). Bracket pairs come from the syntax tree, so brackets in strings and comments do not count; the "cursor" is where the mouse was last pressed in the code.
  - `Diff::overview_ruler(true)`: VS Code's diff overview ruler (`src/diff/overview.rs`).
  - The theme's `highlight.syntax` accepts any capture name (such as `type.builtin`) and falls back by dropping suffixes one at a time (`type.builtin` → `type`); upstream accepted a fixed set of 41 names and fell back only to the first segment.
  - JS/TS/TSX highlight rules are split along VS Code's TextMate scopes: `new` / `typeof` and the like are `keyword.operator`; the ternary operator, the colon of type annotations and `...` are operators; built-in types such as `number` are `type.builtin`; the X in `new X()` is `constructor`; both braces of a template string's `${ }` are `punctuation.special`; JSX component tags are `tag.component`, angle brackets `tag.delimiter`, and `{ }` `punctuation.section.embedded`.
- Menus, button tooltips and notifications are drawn at VS Code 1.117's sizes: upstream uses rem-based presets, and the theme's font size of 13 makes rem 13px too, so the presets come out smaller than VS Code. Menus: 13px text, 24px rows, 6px radius, 26px horizontal padding, 4px vertical padding, 1px separators with 5px above and below (20% of the foreground color, i.e. VS Code's `menu.separatorBackground`), 160px minimum width, 8px outer radius. Tooltips: 12px text, 19px line height, 2px 8px padding, 5px radius; tooltips attached to elements (buttons and so on) follow VS Code's pointer hover: shown below by default, 1px from the trigger, 3px radius, with a small arrow pointing at the trigger (which keeps pointing at it when the window edge pushes the tooltip aside). Notifications: 13px text, 22px line height, 10px 5px padding, 18px icon, 4px radius (position and width are set by the app in `NotificationSettings`).
- `diff::detected_language` is public, so the file editor picks the same highlight language for a path as the diff does.

The first two are saved as `patches/gpui-component-split-scroll.patch`, the third as `patches/gpui-component-highlight-locals.patch`, the fourth as `patches/gpui-component-command-search-icon.patch`, the fifth as `patches/gpui-component-jsx-highlights.patch`, the sixth as `patches/gpui-component-vscode-editor.patch` (apply after the fifth), and the seventh as `patches/gpui-component-vscode-popups.patch`. Then apply `patches/gpui-component-modification-notices.patch`: as Apache 2.0 requires, it marks the top of every changed file as modified. The eighth change, `patches/gpui-component-language-for-path.patch`, goes on last. To upgrade gpui-kit: copy the new gpui-component into `vendor/gpui-component`, run `patch -p1 < ../../patches/<patch file>` for each patch in order in that directory, merge conflicts by hand, then run `cargo test` and the component's own diff tests (`cargo test --lib diff::`; the component's unit tests reference icons and theme files from the upstream repository, so running them on their own needs the upstream directory layout).

`vendor/gpui-base` is a copy of gpui-base 0.9.1 (also Apache 2.0, `vendor/gpui-base/LICENSE-APACHE`), replaced the same way. Its one change, `patches/gpui-base-undo-version.patch`, versions the editor's undo stack the way VS Code's `alternativeVersionId` does: every edit makes a new version and undo / redo return to the version of that step, so a file whose current version equals the one at its last save has no unsaved edits, even after editing and undoing. It adds `InputBaseState::version`, `break_undo_coalescing` (called on save, so that typing afterwards undoes back to the saved version) and `apply_edits` (one undo step that keeps the cursors on their text, for reloading a file changed on disk; the crate's own batch edit puts one cursor at each edit). To upgrade, copy the new gpui-base into `vendor/gpui-base` and apply the patch with `patch -p1` there. Its tests need the upstream layout: copy it to `<dir>/crates/base`, create an empty `<dir>/README.md`, and run `cargo test --features test-support --lib -- input::` in it.

## License

MIT, see `LICENSE`. Third-party works included or adapted (gpui-component, VS Code, tree-sitter-javascript, vscode-icons) keep their own licenses; see `THIRD-PARTY-NOTICES.md`.
