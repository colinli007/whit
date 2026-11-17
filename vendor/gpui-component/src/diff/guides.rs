// Added by Whit, adapted from Visual Studio Code (MIT, Copyright (c) Microsoft
// Corporation); see THIRD-PARTY-NOTICES.md in the Whit repository.
//! VS Code's indentation and bracket pair guides for one side of a file, as
//! drawn with `editor.guides.indentation`, `editor.guides.bracketPairs: true`
//! and the default `editor.guides.bracketPairsHorizontal: "active"`.
//!
//! Follows `guidesTextModelPart.ts` and `indentGuides.ts` of VS Code. Columns
//! are visible columns counted from 0, in display cells: the display text has
//! its tabs expanded already, to VS Code's default tab size of 4.

use gpui::Hsla;
use unicode_width::UnicodeWidthStr as _;

/// `editor.indentSize`, which defaults to the tab size, 4.
const INDENT_SIZE: u32 = 4;

/// The guide colors, which VS Code takes from the theme.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffGuideColors {
    /// `editorIndentGuide.background1`.
    pub indent: Hsla,
    /// `editorIndentGuide.activeBackground1`: the block holding the cursor.
    pub indent_active: Hsla,
    /// `editorBracketHighlight.foreground1` onwards, cycled by nesting level.
    /// The pair holding the cursor uses them as they are, the others at 30%
    /// opacity.
    pub brackets: Vec<Hsla>,
}

/// A bracket token, found by the syntax tree so that brackets in strings and
/// comments are left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bracket {
    pub line: usize,
    pub column: u32,
    pub text: &'static str,
}

/// The opening and closing brackets of JavaScript-like languages; `${` opens
/// a template substitution.
pub(crate) const BRACKET_TOKENS: [&str; 7] = ["(", ")", "[", "]", "{", "}", "${"];

fn closes(open: &str, close: &str) -> bool {
    matches!(
        (open, close),
        ("(", ")") | ("[", "]") | ("{", "}") | ("${", "}")
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Position {
    line: usize,
    column: u32,
}

#[derive(Debug)]
struct BracketPair {
    open: Position,
    /// The column just after the opening bracket.
    open_end: u32,
    close: Position,
    /// How many pairs enclose this one.
    level: u32,
    /// Where the vertical guide runs.
    guide: u32,
    /// The closing bracket follows other text on its line, so the guide runs
    /// through that line and turns along its bottom.
    text_before_close: bool,
}

/// The guides' shape within one line's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuideShape {
    /// A line down the left edge of `column`.
    Vertical,
    /// A line along the row's top from `column` to `end`.
    Top,
    /// A line along the row's bottom from `column` to `end`.
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Guide {
    pub column: u32,
    pub end: u32,
    pub shape: GuideShape,
    pub color: Hsla,
}

/// The guides that follow the cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ActiveGuides {
    /// Lines of the indented block holding the cursor, and its level.
    indent: Option<(std::ops::RangeInclusive<usize>, u32)>,
    /// The innermost bracket pair holding the cursor.
    pair: Option<usize>,
}

#[derive(Debug, Default)]
pub(crate) struct SideGuides {
    /// The number of indent guides on each line.
    levels: Vec<u32>,
    /// Pairs in the order of their opening brackets.
    pairs: Vec<BracketPair>,
    /// `crossing[starts[line]..starts[line + 1]]` are the pairs whose
    /// vertical guide runs through `line`.
    starts: Vec<u32>,
    crossing: Vec<u32>,
}

/// The column of the first non-whitespace character, or `None` for a blank
/// line.
fn indentation(display: &str) -> Option<u32> {
    let content = display.trim_start();
    (!content.is_empty()).then(|| display[..display.len() - content.len()].width() as u32)
}

impl SideGuides {
    /// `lines` are the display texts of one side; `brackets` its bracket
    /// tokens in document order.
    pub(crate) fn new<'a>(
        lines: impl IntoIterator<Item = &'a str>,
        brackets: impl IntoIterator<Item = Bracket>,
    ) -> Self {
        let indents: Vec<Option<u32>> = lines.into_iter().map(indentation).collect();
        let levels = indent_levels(&indents);
        let mut pairs = Vec::new();
        let mut open: Vec<Bracket> = Vec::new();
        for bracket in brackets {
            if !matches!(bracket.text, ")" | "]" | "}") {
                open.push(bracket);
                continue;
            }
            // An unmatched closing bracket is skipped; brackets left open
            // inside a matched pair are dropped with it.
            let Some(depth) = open
                .iter()
                .rposition(|opening| closes(opening.text, bracket.text))
            else {
                continue;
            };
            let opening = open[depth];
            open.truncate(depth);
            pairs.push(pair(&indents, opening, bracket, depth as u32));
        }
        pairs.sort_by_key(|pair| pair.open);

        let mut per_line = vec![Vec::new(); indents.len()];
        for (ix, pair) in pairs.iter().enumerate() {
            let last = pair.close.line + pair.text_before_close as usize;
            for line in pair.open.line + 1..last.min(indents.len()) {
                per_line[line].push(ix as u32);
            }
        }
        let mut starts = Vec::with_capacity(per_line.len() + 1);
        starts.push(0);
        let mut crossing = Vec::new();
        for line in per_line {
            crossing.extend(line);
            starts.push(crossing.len() as u32);
        }
        Self {
            levels,
            pairs,
            starts,
            crossing,
        }
    }

    fn crossing(&self, line: usize) -> &[u32] {
        match (self.starts.get(line), self.starts.get(line + 1)) {
            (Some(start), Some(end)) => &self.crossing[*start as usize..*end as usize],
            _ => &[],
        }
    }

    /// The guides following a cursor at `column` of `line`.
    pub(crate) fn active(&self, line: usize, column: u32) -> ActiveGuides {
        let cursor = Position { line, column };
        // The last pair, in opening order, that strictly contains the cursor
        // is the innermost one.
        let pair = self.pairs.iter().rposition(|pair| {
            let end = Position {
                line: pair.close.line,
                column: pair.close.column + 1,
            };
            pair.open < cursor && cursor < end
        });
        ActiveGuides {
            indent: self.active_indent(line),
            pair,
        }
    }

    /// VS Code's `getActiveIndentGuide`: the block around `line`, or the one
    /// it opens or closes.
    fn active_indent(&self, line: usize) -> Option<(std::ops::RangeInclusive<usize>, u32)> {
        let level = |line: Option<usize>| -> i64 {
            line.and_then(|line| self.levels.get(line))
                .map_or(-1, |level| *level as i64)
        };
        let initial = level(Some(line));
        if initial < 0 {
            return None;
        }
        let (mut start, mut end, indent, mut up, mut down);
        if level(Some(line + 1)) == initial + 1 {
            // The line opens a block: its body is the active one.
            (start, end, indent, up, down) = (line + 1, line + 1, initial + 1, false, true);
        } else if level(line.checked_sub(1)) == initial + 1 {
            // The line closes a block.
            (start, end, indent, up, down) = (line - 1, line - 1, initial + 1, true, false);
        } else if initial == 0 {
            return None;
        } else {
            (start, end, indent, up, down) = (line, line, initial, true, true);
            if level(line.checked_sub(1)) >= indent {
                start = line - 1;
            } else {
                up = false;
            }
            if level(Some(line + 1)) >= indent {
                end = line + 1;
            } else {
                down = false;
            }
        }
        let mut distance = 2;
        while up || down {
            if up {
                match line.checked_sub(distance) {
                    Some(above) if level(Some(above)) >= indent => start = above,
                    _ => up = false,
                }
            }
            if down {
                if level(Some(line + distance)) >= indent {
                    end = line + distance;
                } else {
                    down = false;
                }
            }
            distance += 1;
        }
        Some((start..=end, indent as u32))
    }

    /// The guides of one line, in drawing order.
    pub(crate) fn line_guides(
        &self,
        line: usize,
        active: Option<&ActiveGuides>,
        colors: &DiffGuideColors,
    ) -> Vec<Guide> {
        let active_pair = active.and_then(|active| active.pair);
        let bracket_color = |ix: usize| {
            let color = colors.brackets.get(self.pairs[ix].level as usize % colors.brackets.len().max(1));
            let color = color.copied().unwrap_or_default();
            if active_pair == Some(ix) {
                color
            } else {
                color.opacity(0.3)
            }
        };
        // Sorted by column; a guide under a one-line pair sorts first.
        let mut brackets: Vec<(i64, Guide)> = self
            .crossing(line)
            .iter()
            .map(|ix| {
                let pair = &self.pairs[*ix as usize];
                let guide = Guide {
                    column: pair.guide,
                    end: pair.guide,
                    shape: GuideShape::Vertical,
                    color: bracket_color(*ix as usize),
                };
                (pair.guide as i64, guide)
            })
            .collect();
        if let Some(ix) = active_pair {
            let pair = &self.pairs[ix];
            let color = bracket_color(ix);
            if pair.open.line == pair.close.line {
                if line == pair.open.line {
                    let guide = Guide {
                        column: pair.open_end,
                        end: pair.close.column,
                        shape: GuideShape::Bottom,
                        color,
                    };
                    brackets.push((-1, guide));
                }
            } else {
                if line == pair.open.line && pair.open.column > pair.guide {
                    let guide = Guide {
                        column: pair.guide,
                        end: pair.open.column,
                        shape: GuideShape::Bottom,
                        color,
                    };
                    brackets.push((pair.guide as i64, guide));
                }
                if line == pair.close.line && pair.close.column > pair.guide {
                    let guide = Guide {
                        column: pair.guide,
                        end: pair.close.column,
                        shape: if pair.text_before_close {
                            GuideShape::Bottom
                        } else {
                            GuideShape::Top
                        },
                        color,
                    };
                    brackets.push((pair.guide as i64, guide));
                }
            }
        }
        brackets.sort_by_key(|(key, _)| *key);

        // Lines with a bracket guide never show the active indent guide.
        let active_indent = active
            .and_then(|active| active.indent.clone())
            .filter(|_| brackets.is_empty());
        let mut guides = Vec::with_capacity(brackets.len() + 4);
        let mut queue = brackets.into_iter().peekable();
        for level in 1..=self.levels.get(line).copied().unwrap_or(0) {
            let column = (level - 1) * INDENT_SIZE;
            while let Some((_, guide)) = queue.next_if(|(key, _)| *key < column as i64) {
                guides.push(guide);
            }
            let covered = queue.peek().is_some_and(|(_, guide)| {
                guide.shape == GuideShape::Vertical && guide.column == column
            });
            if !covered {
                let is_active = active_indent
                    .as_ref()
                    .is_some_and(|(lines, active)| lines.contains(&line) && *active == level);
                guides.push(Guide {
                    column,
                    end: column,
                    shape: GuideShape::Vertical,
                    color: if is_active {
                        colors.indent_active
                    } else {
                        colors.indent
                    },
                });
            }
        }
        guides.extend(queue.map(|(_, guide)| guide));
        guides
    }
}

fn pair(indents: &[Option<u32>], open: Bracket, close: Bracket, level: u32) -> BracketPair {
    let open_position = Position {
        line: open.line,
        column: open.column,
    };
    let close_position = Position {
        line: close.line,
        column: close.column,
    };
    // The least indentation of the lines after the opening bracket's, up to
    // and including the closing bracket's.
    let min_indent = indents
        .get(open.line + 1..=close.line)
        .into_iter()
        .flatten()
        .flatten()
        .min()
        .copied()
        .unwrap_or(u32::MAX);
    let text_before_close = indents
        .get(close.line)
        .copied()
        .flatten()
        .is_some_and(|indent| indent < close.column);
    BracketPair {
        open: open_position,
        open_end: open.column + open.text.len() as u32,
        close: close_position,
        level,
        guide: open.column.min(close.column).min(min_indent),
        text_before_close,
    }
}

/// VS Code's `getLinesIndentGuides`: a line's indentation in levels,
/// rounded up; a blank line takes its level from the lines around it.
fn indent_levels(indents: &[Option<u32>]) -> Vec<u32> {
    let mut below = vec![None; indents.len()];
    let mut next = None;
    for (line, indent) in indents.iter().enumerate().rev() {
        below[line] = next;
        if indent.is_some() {
            next = *indent;
        }
    }
    let mut above = None;
    indents
        .iter()
        .zip(below)
        .map(|(indent, below)| match (indent, above, below) {
            (Some(indent), _, _) => {
                above = Some(*indent);
                indent.div_ceil(INDENT_SIZE)
            }
            (None, Some(above), Some(below)) if above < below => 1 + above / INDENT_SIZE,
            (None, Some(above), Some(below)) if above == below => below.div_ceil(INDENT_SIZE),
            (None, Some(_), Some(below)) => 1 + below / INDENT_SIZE,
            // At the top or bottom of the file.
            (None, _, _) => 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors() -> DiffGuideColors {
        DiffGuideColors {
            indent: gpui::rgb(0x505037).into(),
            indent_active: gpui::rgb(0x707057).into(),
            brackets: vec![
                gpui::rgb(0xFFD700).into(),
                gpui::rgb(0xDA70D6).into(),
                gpui::rgb(0x179FFF).into(),
            ],
        }
    }

    /// Brackets found by scanning, for tests whose code has no strings.
    fn guides(code: &str) -> SideGuides {
        let lines: Vec<&str> = code.lines().collect();
        let brackets = lines.iter().enumerate().flat_map(|(line, text)| {
            text.char_indices().filter_map(move |(ix, ch)| {
                let text = BRACKET_TOKENS.into_iter().find(|token| token.starts_with(ch))?;
                Some(Bracket {
                    line,
                    column: ix as u32,
                    text,
                })
            })
        });
        SideGuides::new(lines.iter().copied(), brackets.collect::<Vec<_>>())
    }

    fn shapes(guides: &[Guide]) -> Vec<(u32, u32, GuideShape)> {
        guides
            .iter()
            .map(|guide| (guide.column, guide.end, guide.shape))
            .collect()
    }

    #[test]
    fn blank_lines_take_the_level_of_the_block_they_sit_in() {
        let indents = [Some(0), Some(4), None, Some(4), None, Some(0)];
        assert_eq!(indent_levels(&indents), [0, 1, 1, 1, 1, 0]);
        // Two-space code with indent size 4 still gets one guide per step.
        let indents = [Some(0), Some(2), Some(4), Some(2), Some(0)];
        assert_eq!(indent_levels(&indents), [0, 1, 1, 1, 0]);
    }

    #[test]
    fn a_bracket_guide_replaces_the_indent_guide_at_its_column() {
        let code = "if (x) {\n    a();\n}";
        let side = guides(code);
        // `{` at column 7 and `}` at 0, so the pair's guide runs at column 0,
        // where the indent guide would be.
        let line = side.line_guides(1, None, &colors());
        assert_eq!(shapes(&line), [(0, 0, GuideShape::Vertical)]);
        assert_eq!(line[0].color, colors().brackets[0].opacity(0.3));
        // No vertical guide on the opening line, nor on a closing line that
        // starts with the bracket.
        assert!(side.line_guides(0, None, &colors()).is_empty());
        assert!(side.line_guides(2, None, &colors()).is_empty());
    }

    #[test]
    fn the_pair_holding_the_cursor_turns_along_its_bracket_lines() {
        let side = guides("x = {\n    a: 1,\n};");
        let active = side.active(1, 5);
        let colors = colors();
        // Opening line: along the bottom from the guide to the bracket.
        assert_eq!(
            shapes(&side.line_guides(0, Some(&active), &colors)),
            [(0, 4, GuideShape::Bottom)]
        );
        // The guide is opaque while active.
        let middle = side.line_guides(1, Some(&active), &colors);
        assert_eq!(shapes(&middle), [(0, 0, GuideShape::Vertical)]);
        assert_eq!(middle[0].color, colors.brackets[0]);
        // `}` sits on the guide, so no turn is needed.
        assert!(side.line_guides(2, Some(&active), &colors).is_empty());
    }

    #[test]
    fn a_closing_bracket_after_text_extends_the_guide_through_its_line() {
        let side = guides("x = [\n    1,\n    2];");
        let active = side.active(1, 5);
        let colors = colors();
        // The guide runs at the least indentation inside, column 4; the indent
        // guide keeps column 0.
        assert_eq!(
            shapes(&side.line_guides(1, Some(&active), &colors)),
            [(0, 0, GuideShape::Vertical), (4, 4, GuideShape::Vertical)]
        );
        // Text precedes `]`: the guide runs through its line and turns along
        // the bottom to the bracket.
        assert_eq!(
            shapes(&side.line_guides(2, Some(&active), &colors)),
            [
                (0, 0, GuideShape::Vertical),
                (4, 4, GuideShape::Vertical),
                (4, 5, GuideShape::Bottom)
            ]
        );
    }

    #[test]
    fn a_one_line_pair_holding_the_cursor_is_underlined() {
        let side = guides("call(a, b);");
        let active = side.active(0, 6);
        assert_eq!(
            shapes(&side.line_guides(0, Some(&active), &colors())),
            [(5, 9, GuideShape::Bottom)]
        );
        // Right before the opening bracket is outside the pair.
        assert_eq!(side.active(0, 4).pair, None);
    }

    #[test]
    fn the_active_indent_guide_follows_the_block_of_the_cursor() {
        let code = "a\n    b\n    c\nd\n";
        let side = guides(code);
        let colors = colors();
        // On `a`, which opens the block, the block's guide lights up.
        let active = side.active(0, 0);
        assert_eq!(active.indent, Some((1..=2, 1)));
        let guide = side.line_guides(2, Some(&active), &colors);
        assert_eq!(guide[0].color, colors.indent_active);
        // On `d`, which closes the block, the same block lights up.
        assert_eq!(side.active(3, 0).indent, Some((1..=2, 1)));
    }
}
