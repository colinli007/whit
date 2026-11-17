// Added by Whit, adapted from Visual Studio Code (MIT, Copyright (c) Microsoft
// Corporation); see THIRD-PARTY-NOTICES.md in the Whit repository.
//! VS Code's diff overview ruler: a 30-pixel strip at the right edge with a
//! lane of removed lines and a lane of inserted lines, under a slider that
//! shows and moves the visible part. Follows `overviewRulerFeature.ts`,
//! `overviewZoneManager.ts` and `scrollbarState.ts`.

use std::{cell::Cell, ops::Range, rc::Rc};

use gpui::{
    App, Bounds, Entity, Hitbox, HitboxBehavior, Hsla, IntoElement, ListState, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollEvent, Styled as _,
    canvas, fill, point, px, size,
};
use gpui_base::ScrollbarHandle as _;

use super::{DiffState, state::DisplayRow};

/// `OverviewRulerFeature.ONE_OVERVIEW_WIDTH`; the ruler is two lanes wide.
const LANE_WIDTH: Pixels = px(15.);
pub(crate) const RULER_WIDTH: Pixels = px(30.);
/// `OverviewZoneManager`'s `MINIMUM_HEIGHT`.
const MINIMUM_ZONE_HEIGHT: Pixels = px(4.);
/// `ScrollbarState`'s `MINIMUM_SLIDER_SIZE`.
const MINIMUM_SLIDER_SIZE: Pixels = px(20.);

/// Where the slider was grabbed: the pointer's y and the scroll top then.
pub(crate) type RulerDrag = Rc<Cell<Option<(Pixels, Pixels)>>>;

/// Changed rows of each lane, as fractions of all rows.
pub(crate) struct Zones {
    removed: Vec<Range<f32>>,
    inserted: Vec<Range<f32>>,
}

impl Zones {
    pub(crate) fn new(rows: &[DisplayRow]) -> Self {
        let mut removed = Vec::new();
        let mut inserted = Vec::new();
        let total = rows.len().max(1) as f32;
        let push = |zones: &mut Vec<Range<f32>>, ix: usize| {
            let (start, end) = (ix as f32 / total, (ix + 1) as f32 / total);
            match zones.last_mut() {
                Some(last) if last.end == start => last.end = end,
                _ => zones.push(start..end),
            }
        };
        for (ix, row) in rows.iter().enumerate() {
            if let DisplayRow::Code {
                original,
                modified,
                changed: true,
                ..
            } = row
            {
                if original.is_some() {
                    push(&mut removed, ix);
                }
                if modified.is_some() {
                    push(&mut inserted, ix);
                }
            }
        }
        Self { removed, inserted }
    }
}

pub(crate) struct RulerColors {
    pub removed: Hsla,
    pub inserted: Hsla,
    pub background: Hsla,
    pub slider: Hsla,
    pub slider_hover: Hsla,
    pub slider_active: Hsla,
}

/// The slider's top and height for a scroll position, as VS Code's
/// `ScrollbarState` computes them.
fn slider(visible: Pixels, content: Pixels, scroll_top: Pixels) -> (Pixels, Pixels, f32) {
    if content <= visible {
        return (px(0.), visible, 0.);
    }
    let size = (visible * (visible / content)).floor().max(MINIMUM_SLIDER_SIZE).round();
    let ratio = (visible - size) / (content - visible);
    ((scroll_top * ratio).round(), size, ratio)
}

fn scroll_top(list: &ListState) -> Pixels {
    -list.offset().y
}

fn set_scroll_top(state: &Entity<DiffState>, top: Pixels, cx: &mut App) {
    state.update(cx, |state, cx| {
        let list = state.list();
        let max = (list.content_size().height - list.viewport_bounds().size.height).max(px(0.));
        list.set_offset(point(px(0.), -top.clamp(px(0.), max)));
        cx.notify();
    });
}

pub(crate) fn render_overview_ruler(
    zones: Zones,
    colors: RulerColors,
    state: Entity<DiffState>,
    list: ListState,
    drag: RulerDrag,
) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds: Bounds<Pixels>, hitbox: Hitbox, window, _| {
            let height = bounds.size.height;
            let content = list.content_size().height.max(height);
            let (slider_top, slider_size, ratio) = slider(height, content, scroll_top(&list));

            window.paint_quad(fill(bounds, colors.background));
            for (zones, left, color) in [
                (&zones.removed, px(0.), colors.removed),
                (&zones.inserted, LANE_WIDTH, colors.inserted),
            ] {
                for zone in zones {
                    let y1 = (height * zone.start).floor();
                    let y2 = (height * zone.end).floor();
                    let half = ((y2 - y1) / 2.).max(MINIMUM_ZONE_HEIGHT / 2.);
                    let center = ((y1 + y2) / 2.).clamp(half, height - half);
                    window.paint_quad(fill(
                        Bounds::new(
                            bounds.origin + point(left, center - half),
                            size(LANE_WIDTH, half * 2.),
                        ),
                        color,
                    ));
                }
            }
            let slider_bounds = Bounds::new(
                bounds.origin + point(px(0.), slider_top),
                size(RULER_WIDTH, slider_size),
            );
            let slider_color = if drag.get().is_some() {
                colors.slider_active
            } else if hitbox.is_hovered(window) && slider_bounds.contains(&window.mouse_position())
            {
                colors.slider_hover
            } else {
                colors.slider
            };
            window.paint_quad(fill(slider_bounds, slider_color));

            window.on_mouse_event({
                let (state, list, drag) = (state.clone(), list.clone(), drag.clone());
                let hitbox = hitbox.clone();
                move |event: &MouseDownEvent, phase, window, cx| {
                    if !phase.bubble()
                        || event.button != MouseButton::Left
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }
                    let y = event.position.y - bounds.top();
                    let mut top = scroll_top(&list);
                    // Pressing beside the slider centers it there first.
                    if (y < slider_top || y >= slider_top + slider_size) && ratio > 0. {
                        top = (y - slider_size / 2.) / ratio;
                        set_scroll_top(&state, top, cx);
                        top = scroll_top(&list);
                    }
                    list.start_drag();
                    drag.set(Some((event.position.y, top)));
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event({
                let (state, drag) = (state.clone(), drag.clone());
                move |event: &MouseMoveEvent, phase, _, cx| {
                    if let (true, Some((start_y, start_top))) = (phase.bubble(), drag.get())
                        && ratio > 0.
                    {
                        set_scroll_top(&state, start_top + (event.position.y - start_y) / ratio, cx);
                    }
                }
            });
            window.on_mouse_event({
                let (state, list, drag) = (state.clone(), list.clone(), drag.clone());
                move |_: &MouseUpEvent, phase, _, cx| {
                    if phase.bubble() && drag.take().is_some() {
                        list.end_drag();
                        state.update(cx, |_, cx| cx.notify());
                    }
                }
            });
            window.on_mouse_event({
                let (state, list) = (state.clone(), list.clone());
                move |event: &ScrollEvent, phase, window, cx| {
                    if phase.bubble() && hitbox.is_hovered(window) {
                        let delta = event.delta.pixel_delta(window.line_height());
                        set_scroll_top(&state, scroll_top(&list) - delta.y, cx);
                    }
                }
            });
        },
    )
    .w(RULER_WIDTH)
    .h_full()
    .flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_rows_become_zones_in_their_lanes() {
        let code = |original, modified, changed| DisplayRow::Code {
            file: 0,
            original,
            modified,
            changed,
        };
        let rows = [
            code(Some(0), Some(0), false),
            // A modified line: both lanes.
            code(Some(1), Some(1), true),
            // Then a removed line and an inserted one; adjacent rows merge.
            code(Some(2), None, true),
            code(None, Some(2), true),
        ];
        let zones = Zones::new(&rows);
        assert_eq!(zones.removed, [0.25..0.75]);
        assert_eq!(zones.inserted, [0.25..0.5, 0.75..1.0]);
    }

    #[test]
    fn the_slider_spans_the_visible_part_and_keeps_a_grabbable_size() {
        // Everything fits: the slider fills the ruler and cannot move.
        assert_eq!(slider(px(500.), px(400.), px(0.)), (px(0.), px(500.), 0.));
        // A quarter visible, scrolled to the end.
        let (top, size, _) = slider(px(400.), px(1600.), px(1200.));
        assert_eq!((top, size), (px(300.), px(100.)));
        // Very long content still gets a 20-pixel slider.
        assert_eq!(slider(px(400.), px(400_000.), px(0.)).1, px(20.));
    }
}
