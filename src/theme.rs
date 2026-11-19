//! VS Code's Monokai Dimmed colors with its Fira Code editor font, fixed
//! regardless of the system appearance. Colors the theme leaves unset are VS
//! Code's dark defaults.
use std::rc::Rc;

use gpui_kit::{
    Anchor, App,
    component::{
        notification::NotificationSettings,
        theme::{Theme, ThemeMode, ThemeSet},
    },
    px,
};

const MONOKAI_DIMMED: &str = include_str!("../assets/themes/monokai-dimmed.json");

pub fn init(cx: &mut App) {
    let config = serde_json::from_str::<ThemeSet>(MONOKAI_DIMMED)
        .expect("the bundled theme parses")
        .themes
        .into_iter()
        .next()
        .expect("the bundled theme set holds a theme");
    Theme::global_mut(cx).apply_config(&Rc::new(config));
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::global_mut(cx).notification = vscode_toasts();
}

/// VS Code's toasts: 450 pixels wide at the bottom right, 3 pixels in from
/// the window's edge and above the 22-pixel status bar, each with a 4-pixel
/// margin.
fn vscode_toasts() -> NotificationSettings {
    let mut toasts = NotificationSettings::default();
    toasts.placement = Anchor::BottomRight;
    toasts.margins.right = px(3. + 4.);
    toasts.margins.bottom = px(22. + 3. + 4.);
    toasts.width = px(450.);
    toasts
}

/// Colors with no slot in the component theme: Monokai Dimmed's own, or VS
/// Code's dark defaults from its color registry where the theme sets none.
pub mod colors {
    use gpui_kit::component::diff::{DiffChangeColors, DiffGuideColors};
    use gpui_kit::{Hsla, rgb, rgba};

    /// The indentation and bracket pair guides: Monokai Dimmed's
    /// `editorIndentGuide.*`, and VS Code's default
    /// `editorBracketHighlight.foreground1` to `3`.
    pub fn guides() -> DiffGuideColors {
        DiffGuideColors {
            indent: rgb(0x505037).into(),
            indent_active: rgb(0x707057).into(),
            brackets: vec![
                rgb(0xFFD700).into(),
                rgb(0xDA70D6).into(),
                rgb(0x179FFF).into(),
            ],
        }
    }

    /// The `diffEditor.*` line and text backgrounds.
    pub fn diff() -> DiffChangeColors {
        DiffChangeColors {
            inserted_line: rgba(0x9BB95533).into(),
            removed_line: rgba(0xFF000033).into(),
            inserted_text: rgba(0x9CCC2C33).into(),
            removed_text: rgba(0xFF000033).into(),
        }
    }

    /// `gitDecoration.modifiedResourceForeground`.
    pub fn modified() -> Hsla {
        rgb(0xE2C08D).into()
    }
    /// `gitDecoration.addedResourceForeground`.
    pub fn added() -> Hsla {
        rgb(0x81B88B).into()
    }
    /// `gitDecoration.deletedResourceForeground`.
    pub fn deleted() -> Hsla {
        rgb(0xC74E39).into()
    }
    /// `gitDecoration.untrackedResourceForeground`, also used for renames.
    pub fn untracked() -> Hsla {
        rgb(0x73C991).into()
    }
    /// `gitDecoration.conflictingResourceForeground`.
    pub fn conflicting() -> Hsla {
        rgb(0xE4676B).into()
    }

    /// `input.background`.
    pub fn input_background() -> Hsla {
        rgb(0x3C3C3C).into()
    }
    /// `badge.background`.
    pub fn badge_background() -> Hsla {
        rgb(0x4D4D4D).into()
    }
    /// `badge.foreground`.
    pub fn badge_foreground() -> Hsla {
        rgb(0xFFFFFF).into()
    }
    /// `tab.border`, between a tab and the next.
    pub fn tab_border() -> Hsla {
        rgb(0x303030).into()
    }
    /// `list.activeSelectionBackground`: the selected row of a focused list.
    /// The theme's `list.active` is the unfocused one.
    pub fn list_focused_selection() -> Hsla {
        rgb(0x707070).into()
    }
    /// `list.focusOutline`, which is `focusBorder`.
    pub fn list_focus_outline() -> Hsla {
        rgb(0x3655B5).into()
    }
    /// `list.highlightForeground`: the characters a filter matched.
    pub fn list_highlight() -> Hsla {
        rgb(0xE58520).into()
    }
    /// `quickInput.background`, which is `editorWidget.background`.
    pub fn quick_input_background() -> Hsla {
        rgb(0x252526).into()
    }
    /// `activityBar.background` of Monokai Dimmed.
    pub fn activity_bar() -> Hsla {
        rgb(0x353535).into()
    }
    /// `activityBar.foreground`: the active view's icon and its marker.
    pub fn activity_bar_foreground() -> Hsla {
        rgb(0xFFFFFF).into()
    }
    /// `activityBar.inactiveForeground`, VS Code's default of the foreground
    /// at 40%.
    pub fn activity_bar_inactive() -> Hsla {
        rgba(0xFFFFFF66).into()
    }
    /// `activityBarTop.foreground`, VS Code's dark default: the active
    /// view's icon and its line when the activity bar lies across the side
    /// bar.
    pub fn activity_bar_top_foreground() -> Hsla {
        rgb(0xE7E7E7).into()
    }
    /// `activityBarTop.inactiveForeground`, the foreground at 60%.
    pub fn activity_bar_top_inactive() -> Hsla {
        rgba(0xE7E7E799).into()
    }
    /// `settings.headerForeground`: setting titles and group headings.
    pub fn settings_header_foreground() -> Hsla {
        rgb(0xE7E7E7).into()
    }
    /// `settings.rowHoverBackground`: `list.hoverBackground` at 30%.
    pub fn settings_row_hover() -> Hsla {
        rgba(0x4444444D).into()
    }
    /// `settings.focusedRowBackground`: `list.hoverBackground` at 60%.
    pub fn settings_focused_row() -> Hsla {
        rgba(0x44444499).into()
    }
    /// `settings.focusedRowBorder`, which is `focusBorder`.
    pub fn focus_border() -> Hsla {
        rgb(0x3655B5).into()
    }
    /// `settings.modifiedItemIndicator`, VS Code's dark default.
    pub fn settings_modified() -> Hsla {
        rgb(0x0C7D9D).into()
    }
    /// `settings.dropdownBackground` and its border, which follow
    /// `dropdown.background` of Monokai Dimmed; checkboxes take the same.
    pub fn settings_control() -> Hsla {
        rgb(0x525252).into()
    }
    /// `dropdown.foreground`, VS Code's dark default.
    pub fn settings_control_foreground() -> Hsla {
        rgb(0xF0F0F0).into()
    }
    /// `settings.headerBorder`, which is `panel.border`.
    pub fn settings_header_border() -> Hsla {
        rgba(0x80808059).into()
    }
    /// `activityBarBadge.background` of Monokai Dimmed.
    pub fn activity_badge() -> Hsla {
        rgb(0x3655B5).into()
    }
    /// `gitDecoration.ignoredResourceForeground`.
    pub fn ignored() -> Hsla {
        rgb(0x8C8C8C).into()
    }
    /// `tree.indentGuidesStroke`.
    pub fn tree_indent_guide() -> Hsla {
        rgb(0x585858).into()
    }
    /// `sideBarSectionHeader.background` of Monokai Dimmed.
    pub fn side_bar_section_header() -> Hsla {
        rgb(0x505050).into()
    }

    /// `widget.shadow`; dark themes draw no widget border.
    pub fn widget_shadow() -> Hsla {
        rgba(0x0000005C).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::{TestAppContext, rgb};

    #[gpui_kit::test]
    fn the_window_uses_monokai_dimmed_with_fira_code(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            let theme = Theme::global(cx);
            assert!(theme.mode.is_dark());
            assert_eq!(theme.background, rgb(0x1E1E1E).into());
            assert_eq!(theme.sidebar, rgb(0x272727).into());
            assert_eq!(theme.status_bar, rgb(0x505050).into());
            assert_eq!(theme.tab_bar, rgb(0x282828).into());
            assert_eq!(theme.foreground, rgb(0xCCCCCC).into());
            assert_eq!(theme.primary, rgb(0x565656).into());
            assert_eq!(theme.mono_font_family, "Fira Code");
            assert_eq!(theme.mono_font_size, gpui_kit::px(14.));
        });
    }

    /// Checks the color the bundled theme gives the first occurrence of each
    /// token, reporting every token at once.
    fn assert_colors(
        cx: &mut TestAppContext,
        language: &str,
        code: &str,
        expected: &[(&str, u32)],
    ) {
        use gpui_kit::component::{Rope, highlighter::SyntaxHighlighter};

        let mut highlighter = SyntaxHighlighter::new(language);
        highlighter.update(None, &Rope::from(code), None);
        let actual: Vec<_> = cx.update(|cx| {
            crate::init(cx);
            let theme = Theme::global(cx).highlight_theme.clone();
            let styles = highlighter.styles(&(0..code.len()), &*theme);
            expected
                .iter()
                .map(|(token, _)| {
                    let start = code.find(token).unwrap();
                    let color = styles
                        .iter()
                        .find(|(range, _)| range.start <= start && start < range.end)
                        .and_then(|(_, style)| style.color);
                    (*token, color)
                })
                .collect()
        });
        let expected: Vec<_> = expected
            .iter()
            .map(|(token, color)| (*token, Some(rgb(*color).into())))
            .collect();
        assert_eq!(actual, expected);
    }

    #[gpui_kit::test]
    fn typescript_is_highlighted_in_monokai_dimmed_colors(cx: &mut TestAppContext) {
        let code = "// note\nexport const toPath = (stroke: number) => \"M\" + 1;\n\
                    const shape = new Path2D(`M${stroke}`);";
        assert_colors(
            cx,
            "typescript",
            code,
            &[
                ("// note", 0x9A9B99),
                ("const", 0x9872A2),
                ("toPath", 0xCE6700),
                ("stroke", 0x6089B4),
                // Built-in types are `support.type`, red; declared ones darker.
                ("number", 0xC7444A),
                ("=>", 0x676867),
                ("\"M\"", 0x9AA83A),
                ("1", 0x6089B4),
                // `new` is `keyword.operator`, gray like the operators.
                ("new", 0x676867),
                ("Path2D", 0xCE6700),
                ("${", 0xD08442),
                // Inside a substitution, code keeps its own colors.
                ("stroke}", 0x6089B4),
                ("}`", 0xD08442),
            ],
        );
    }

    #[gpui_kit::test]
    fn tsx_is_highlighted_like_typescript_plus_jsx(cx: &mut TestAppContext) {
        let code = "const node = <path d={toPath(stroke, 1)} fill=\"none\" />;\n\
                    const icon = <IconFont size={SIZE} />;";
        assert_colors(
            cx,
            "tsx",
            code,
            &[
                ("const", 0x9872A2),
                ("<", 0x6089B4),
                ("path", 0x6089B4),
                ("d=", 0xD0B344),
                ("{", 0xD0B344),
                ("toPath", 0xCE6700),
                ("1", 0x6089B4),
                ("\"none\"", 0x9AA83A),
                ("IconFont", 0xC7444A),
                ("SIZE", 0x6089B4),
            ],
        );
    }

    #[gpui_kit::test]
    fn jsx_in_javascript_is_highlighted(cx: &mut TestAppContext) {
        let code = "const el = <div className=\"a\">{ok ? 1 : 2}</div>;";
        assert_colors(
            cx,
            "javascript",
            code,
            &[
                ("div", 0x6089B4),
                ("className", 0xD0B344),
                ("{", 0xD0B344),
                ("ok", 0x6089B4),
                ("?", 0x676867),
                (":", 0x676867),
            ],
        );
    }
}
