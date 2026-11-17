// Modified by Whit; its patches/ folder holds the changes.
use gpui::{
    AppContext as _, Context, Entity, IntoElement, Modifiers, MouseButton, ParentElement as _,
    Pixels, Render, ScrollDelta, ScrollEvent, Styled as _, TestAppContext, VisualTestContext,
    Window, div, point, px,
};

use super::{Diff, DiffMode, DiffSide, DiffState, document::fixture};

/// Every line changed, so the unified patch is two rows per line.
const LINES: usize = 300;

struct Host {
    state: Entity<DiffState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(400.))
            .h(px(200.))
            // The application root provides this; pointer selection needs it.
            .child(gpui_base::TextSelectionLayer)
            .child(Diff::new(&self.state).size_full())
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
}

fn long_patch(cx: &mut TestAppContext) -> (Entity<DiffState>, &mut VisualTestContext) {
    cx.update(crate::init);
    let original = (0..LINES)
        .map(|ix| format!("old {ix}\n"))
        .collect::<String>();
    let modified = (0..LINES)
        .map(|ix| format!("new {ix}\n"))
        .collect::<String>();
    let state =
        cx.new(|cx| DiffState::new([fixture::modified("long.txt", &original, &modified)], cx));
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| Host { state }
    });
    // The body records its size on the first paint and lays out again.
    for _ in 0..3 {
        draw(cx);
    }
    (state, cx)
}

#[gpui::test]
fn the_whole_patch_keeps_its_height_after_the_first_layout(cx: &mut TestAppContext) {
    let (state, cx) = long_patch(cx);
    let (max, viewport) = cx.update(|_, cx| {
        let list = state.read(cx).list();
        (
            list.max_offset_for_scrollbar().y,
            list.viewport_bounds().size.height,
        )
    });
    // Rows past the viewport have not been rendered; without their hinted
    // height the patch would read as barely taller than one screen.
    assert!(
        max > viewport * 10.,
        "max offset {max:?} for a {viewport:?} viewport"
    );
}

#[gpui::test]
fn the_wheel_scrolls_past_the_rows_rendered_so_far(cx: &mut TestAppContext) {
    let (state, cx) = long_patch(cx);
    for _ in 0..50 {
        cx.simulate_event(ScrollEvent {
            position: point(px(100.), px(120.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-40.))),
            ..Default::default()
        });
        draw(cx);
    }
    let top = cx.update(|_, cx| state.read(cx).list().logical_scroll_top().item_ix);
    // 2000 px of wheel travel crosses far more than the first screen of rows.
    assert!(top > 40, "scrolled to row {top}");
}

/// Numbered tokens tagged by side, so a selection shows which side and which
/// part of the line it covers.
fn wide_line(tag: char) -> String {
    (0..200).map(|ix| format!("{tag}{ix:03} ")).collect()
}

/// One changed line far wider than either split column of a 400 px body.
fn wide_split(cx: &mut TestAppContext) -> (Entity<DiffState>, &mut VisualTestContext) {
    cx.update(crate::init);
    let original = format!("{}\n", wide_line('o'));
    let modified = format!("{}\n", wide_line('n'));
    let state = cx.new(|cx| {
        DiffState::new([fixture::modified("wide.txt", &original, &modified)], cx)
            .with_mode(DiffMode::Split)
    });
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| Host { state }
    });
    for _ in 0..3 {
        draw(cx);
    }
    (state, cx)
}

/// Where each side's line starts and ends in the window, scrolled out of view or not.
fn line_extent(
    state: &Entity<DiffState>,
    side: DiffSide,
    cx: &mut VisualTestContext,
) -> (Pixels, Pixels) {
    cx.update(|_, cx| {
        let runs = state.read(cx).geometry().borrow().run_bounds();
        let runs = runs.iter().filter(|(run_side, _)| *run_side == side);
        let start = runs
            .clone()
            .map(|(_, bounds)| bounds.left())
            .fold(Pixels::MAX, Pixels::min);
        let end = runs
            .map(|(_, bounds)| bounds.right())
            .fold(Pixels::MIN, Pixels::max);
        (start, end)
    })
}

fn scroll_sideways(delta: Pixels, cx: &mut VisualTestContext) {
    cx.simulate_event(ScrollEvent {
        position: point(px(100.), px(100.)),
        delta: ScrollDelta::Pixels(point(delta, px(0.))),
        ..Default::default()
    });
    draw(cx);
}

#[gpui::test]
fn split_columns_clip_long_lines_and_scroll_sideways_together(cx: &mut TestAppContext) {
    let (state, cx) = wide_split(cx);
    let (old_start, _) = line_extent(&state, DiffSide::Original, cx);
    let (new_start, _) = line_extent(&state, DiffSide::Modified, cx);
    // The modified column starts mid-body, not past the longest line.
    assert!(old_start < px(100.), "original starts at {old_start:?}");
    assert!(
        new_start > px(200.) && new_start < px(300.),
        "modified starts at {new_start:?}"
    );

    scroll_sideways(px(-100.), cx);
    assert_eq!(
        line_extent(&state, DiffSide::Original, cx).0,
        old_start - px(100.)
    );
    assert_eq!(
        line_extent(&state, DiffSide::Modified, cx).0,
        new_start - px(100.)
    );

    // Scrolling stops once the line's end reaches the column's edge.
    for _ in 0..100 {
        scroll_sideways(px(-200.), cx);
    }
    let (_, old_end) = line_extent(&state, DiffSide::Original, cx);
    let (_, new_end) = line_extent(&state, DiffSide::Modified, cx);
    assert!(
        old_end > px(180.) && old_end <= px(200.),
        "original ends at {old_end:?}"
    );
    assert!(
        new_end > px(380.) && new_end <= px(400.),
        "modified ends at {new_end:?}"
    );
}

#[gpui::test]
fn dragging_after_a_sideways_scroll_selects_the_text_under_the_pointer(cx: &mut TestAppContext) {
    let (state, cx) = wide_split(cx);
    scroll_sideways(px(-300.), cx);
    let row_y = cx.update(|_, cx| {
        let runs = state.read(cx).geometry().borrow().run_bounds();
        runs[0].1.center().y
    });
    // Each column's clipped text runs on, unseen, under the other column.
    for (side, tag, from, to) in [
        (DiffSide::Original, 'o', px(60.), px(140.)),
        (DiffSide::Modified, 'n', px(260.), px(340.)),
    ] {
        cx.simulate_mouse_down(point(from, row_y), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(point(to, row_y), Some(MouseButton::Left), Modifiers::none());
        cx.simulate_mouse_up(point(to, row_y), MouseButton::Left, Modifiers::none());
        draw(cx);

        let line = wide_line(tag);
        let (start, end) = line_extent(&state, side, cx);
        let char_width = (end - start) / line.len() as f32;
        let selected = cx.update(|_, cx| state.read(cx).selected_text(cx));
        assert!(selected.len() >= 8, "{side:?} selected {selected:?}");
        let found = line
            .find(&selected)
            .unwrap_or_else(|| panic!("{side:?} selected {selected:?}"));
        let expected = ((from - start) / char_width).round() as usize;
        assert!(
            found.abs_diff(expected) <= 1,
            "{side:?} selected {selected:?} at {found}, pointer over {expected}"
        );
    }
}
