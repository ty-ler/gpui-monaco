//! Where to scroll to reveal a range.
//!
//! Ports `_computeScrollTopToRevealRange` and `_computeMinimumScrolling` from
//! Monaco's `src/vs/editor/browser/viewParts/viewLines/viewLines.ts` (MIT),
//! in pixels, and Monaco's `ScrollType` (`editorCommon.ts`).

/// Whether a scroll animates (Monaco `ScrollType`).
///
/// Monaco's reveal methods default to [`ScrollType::Smooth`] and its
/// `setScrollTop`/`setScrollLeft`/`setScrollPosition` to
/// [`ScrollType::Immediate`]; the methods without a scroll type here do
/// the same. Smooth scrolls animate only with the `smooth_scrolling`
/// option, and reveals of a line or less jump, as in Monaco.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollType {
    /// Animate to the new position when `smooth_scrolling` is on.
    #[default]
    Smooth,
    /// Jump to the new position, stopping any animation.
    Immediate,
}

/// How a range is revealed vertically (Monaco `VerticalRevealType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RevealType {
    /// Scroll as little as possible (Monaco `revealLine`, `revealRange`).
    #[default]
    Simple,
    /// Center the range.
    Center,
    /// Center the range only if it's not already visible.
    CenterIfOutsideViewport,
    /// Put the range at the top.
    Top,
    /// Put the range at the bottom.
    Bottom,
    /// Put the range near the top (20% of the viewport, at least five lines,
    /// above it).
    NearTop,
    /// Like [`RevealType::NearTop`], but only if it's not already visible.
    NearTopIfOutsideViewport,
}

/// The inputs for [`scroll_top_to_reveal`], in pixels.
#[derive(Clone, Copy, Debug)]
pub struct RevealRequest {
    /// The current scroll top.
    pub viewport_top: f32,
    /// The text area's height.
    pub viewport_height: f32,
    /// The top of the range's first line.
    pub box_start: f32,
    /// The bottom of the range's last line.
    pub box_end: f32,
    /// The line height.
    pub line_height: f32,
    /// Context lines kept around the range: Monaco uses the sticky scroll
    /// line limit when sticky scroll is on (`cursorSurroundingLines`
    /// otherwise, default 0).
    pub surrounding_lines: f32,
    /// A mouse gesture moved the cursor (Monaco's `source === 'mouse'`
    /// with `cursorSurroundingLinesStyle: default`): no context lines, but
    /// one line of room above.
    pub mouse: bool,
    /// Monaco's `minimalReveal` (`revealAllCursors(_, true)`): no extra
    /// line of room above or below. Callers also set `mouse` for it with
    /// the default `cursorSurroundingLinesStyle`, which drops the context
    /// lines.
    pub minimal: bool,
}

/// Monaco `_computeMinimumScrolling`.
pub(crate) fn minimum_scrolling(
    viewport_start: f32,
    viewport_end: f32,
    box_start: f32,
    box_end: f32,
    at_start: bool,
    at_end: bool,
) -> f32 {
    let viewport_length = viewport_end - viewport_start;
    let box_length = box_end - box_start;
    if box_length < viewport_length {
        if at_start {
            return box_start;
        }
        if at_end {
            return (box_end - viewport_length).max(0.);
        }
        if box_start < viewport_start {
            return box_start;
        } else if box_end > viewport_end {
            return (box_end - viewport_length).max(0.);
        }
    } else {
        // The box is larger than the viewport: show its start.
        return box_start;
    }
    viewport_start
}

/// The scroll top that reveals the request's box (Monaco
/// `_computeScrollTopToRevealRange` for a single range).
pub fn scroll_top_to_reveal(request: RevealRequest, reveal: RevealType) -> f32 {
    scroll_top_to_reveal_box(request, reveal, true).unwrap_or(request.viewport_top)
}

/// The scroll top that reveals the box around several cursors (Monaco
/// `_computeScrollTopToRevealRange` with `selections`, from
/// `revealAllCursors`), or `None` when they don't fit in the viewport:
/// Monaco then doesn't scroll.
pub fn scroll_top_to_reveal_cursors(request: RevealRequest, reveal: RevealType) -> Option<f32> {
    scroll_top_to_reveal_box(request, reveal, false)
}

fn scroll_top_to_reveal_box(
    request: RevealRequest,
    reveal: RevealType,
    single_range: bool,
) -> Option<f32> {
    let RevealRequest {
        viewport_top,
        viewport_height,
        mut box_start,
        mut box_end,
        line_height,
        surrounding_lines,
        mouse,
        minimal,
    } = request;
    let viewport_end = viewport_top + viewport_height;
    let (padding_top, mut padding_bottom) = if mouse {
        // Reveal one more line above (when dragging).
        (if minimal { 0. } else { line_height }, 0.)
    } else {
        let context = (viewport_height / line_height / 2.).min(surrounding_lines);
        (context * line_height, (context - 1.).max(0.) * line_height)
    };
    if !minimal && matches!(reveal, RevealType::Simple | RevealType::Bottom) {
        // One more line where the horizontal scrollbar may cover it.
        padding_bottom += line_height;
    }
    box_start -= padding_top;
    box_end += padding_bottom;

    if box_end - box_start > viewport_height {
        // Don't reveal more cursors than fit.
        return single_range.then_some(box_start.max(0.));
    }
    let inside = viewport_top <= box_start && box_end <= viewport_end;
    let top = match reveal {
        RevealType::NearTop | RevealType::NearTopIfOutsideViewport => {
            if reveal == RevealType::NearTopIfOutsideViewport && inside {
                viewport_top
            } else {
                let gap = (5. * line_height).max(viewport_height * 0.2);
                (box_start - gap).max(box_end - viewport_height)
            }
        }
        RevealType::Center | RevealType::CenterIfOutsideViewport => {
            if reveal == RevealType::CenterIfOutsideViewport && inside {
                viewport_top
            } else {
                ((box_start + box_end) / 2. - viewport_height / 2.).max(0.)
            }
        }
        _ => minimum_scrolling(
            viewport_top,
            viewport_end,
            box_start,
            box_end,
            reveal == RevealType::Top,
            reveal == RevealType::Bottom,
        ),
    };
    Some(top.max(0.))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(top: f32, line: f32) -> RevealRequest {
        RevealRequest {
            viewport_top: top,
            viewport_height: 200.,
            box_start: line * 10.,
            box_end: line * 10. + 10.,
            line_height: 10.,
            surrounding_lines: 0.,
            mouse: false,
            minimal: false,
        }
    }

    #[test]
    fn simple_scrolls_minimally() {
        // Visible: no change.
        assert_eq!(
            scroll_top_to_reveal(request(0., 5.), RevealType::Simple),
            0.
        );
        // Below: just enough, plus one line for the scrollbar.
        assert_eq!(
            scroll_top_to_reveal(request(0., 30.), RevealType::Simple),
            120.
        );
        // Above: to its top.
        assert_eq!(
            scroll_top_to_reveal(request(500., 30.), RevealType::Simple),
            300.
        );
    }

    #[test]
    fn center_top_bottom() {
        assert_eq!(
            scroll_top_to_reveal(request(0., 50.), RevealType::Center),
            405.
        );
        // Already visible: stays.
        assert_eq!(
            scroll_top_to_reveal(request(0., 5.), RevealType::CenterIfOutsideViewport),
            0.
        );
        assert_eq!(
            scroll_top_to_reveal(request(0., 50.), RevealType::Top),
            500.
        );
        assert_eq!(
            scroll_top_to_reveal(request(0., 50.), RevealType::Bottom),
            320.
        );
        // Near top: a 20% (min five lines) gap above.
        assert_eq!(
            scroll_top_to_reveal(request(0., 50.), RevealType::NearTop),
            450.
        );
    }

    #[test]
    fn surrounding_lines_pad_the_box() {
        let mut r = request(0., 30.);
        r.surrounding_lines = 5.;
        // Five lines above, four (plus one) below.
        assert_eq!(scroll_top_to_reveal(r, RevealType::Simple), 160.);
    }

    #[test]
    fn minimal_reveal_adds_no_room() {
        // Monaco `minimalReveal` with the default surrounding lines style.
        let mut r = request(0., 30.);
        r.mouse = true;
        r.minimal = true;
        // Just the line itself: no line of room above or below.
        assert_eq!(scroll_top_to_reveal(r, RevealType::Simple), 110.);
        r.viewport_top = 500.;
        assert_eq!(scroll_top_to_reveal(r, RevealType::Simple), 300.);
    }

    #[test]
    fn cursor_boxes_larger_than_the_viewport_dont_scroll() {
        // Lines 10 to 40 (310px with the extra line) don't fit in 200px.
        let mut r = request(0., 10.);
        r.box_end = 410.;
        assert_eq!(scroll_top_to_reveal_cursors(r, RevealType::Simple), None);
        // A single range scrolls to its start instead.
        assert_eq!(scroll_top_to_reveal(r, RevealType::Simple), 100.);
        // Lines 30 to 32 fit.
        let mut r = request(0., 30.);
        r.box_end = 330.;
        assert_eq!(
            scroll_top_to_reveal_cursors(r, RevealType::Simple),
            Some(140.)
        );
    }
}
