//! Snap-to-element — the "smart lasso" the screen pickers offer in place of a
//! hand-drawn box.
//!
//! Point at a button and this returns the button's rectangle. Drawing a tight
//! box by hand is the fiddliest part of setting up a template or a surgical
//! click: a few pixels of the surrounding panel dragged in with it is enough to
//! stop the template matching once the panel scrolls, and a few pixels short
//! clips the glyph the match keys on.
//!
//! The method is a region grow from the pixel under the cursor, with two
//! tolerances rather than one. A pixel joins if it is close to the neighbour it
//! was reached from (so a gradient-filled control is followed all the way across)
//! *and* still within a wider distance of the seed (so the grow cannot drift
//! through a soft edge into the panel behind). This is the rule a magic-wand
//! tool uses, and it is what makes the difference on game UI, which is nearly all
//! gradients and soft borders.
//!
//! Everything here is pure arithmetic over a [`Frame`], so it is tested against
//! synthetic frames rather than the screen.

use std::collections::VecDeque;

use super::capture::Frame;

/// A rectangle in frame pixels, `(x, y, w, h)`.
pub type Rect = (i32, i32, i32, i32);

/// How far a pixel may sit from the neighbour it was reached from. Small, so a
/// border stroke or a glyph edge stops the grow.
const LOCAL: i32 = 14;

/// How far a pixel may sit from the seed however gradually it got there. This is
/// what bounds a gradient: a control may shade across this much and no further.
const GLOBAL: i32 = 72;

/// The window searched around the cursor. A control larger than this is not what
/// the lasso is for, and an unbounded grow across a flat desktop would sweep the
/// whole screen before finding out it had nothing.
const SEARCH_W: i32 = 800;
const SEARCH_H: i32 = 480;

/// Smallest snap worth offering. Below this the cursor is over a gap or a hair
/// line, and a wrong box costs the user more than no box.
const MIN_SIDE: i32 = 14;

/// A result narrower than this on either side is more likely a letter of a
/// button's label than the button, so what encloses it is tried as well.
const GLYPH: i32 = 26;

/// Per-channel distance — the same max-abs metric the guard colour match uses,
/// which is what makes a snapped region and a colour guard agree about what
/// counts as "the same shade".
fn dist(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())
}

fn pixel(frame: &Frame, x: i32, y: i32) -> [i32; 3] {
    let i = (y as usize * frame.width as usize + x as usize) * 3;
    [i32::from(frame.bgr[i]), i32::from(frame.bgr[i + 1]), i32::from(frame.bgr[i + 2])]
}

fn area((_, _, w, h): Rect) -> i64 {
    i64::from(w) * i64::from(h)
}

fn contains(outer: Rect, inner: Rect) -> bool {
    let (ox, oy, ow, oh) = outer;
    let (ix, iy, iw, ih) = inner;
    ox <= ix && oy <= iy && ox + ow >= ix + iw && oy + oh >= iy + ih
}

/// Grow from `(sx, sy)` and return the bounding box of everything reached.
///
/// `None` when the grow ran into the edge of its own search window without the
/// frame ending there, or ate most of the window: both mean the seed was on a
/// background that carries on past anything we can call an element, and offering
/// a box the size of the search window would be worse than offering none.
fn grow(frame: &Frame, sx: i32, sy: i32) -> Option<Rect> {
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    if sx < 0 || sy < 0 || sx >= fw || sy >= fh {
        return None;
    }

    let x0 = (sx - SEARCH_W / 2).max(0);
    let y0 = (sy - SEARCH_H / 2).max(0);
    let x1 = (sx + SEARCH_W / 2).min(fw);
    let y1 = (sy + SEARCH_H / 2).min(fh);
    let (ww, wh) = (x1 - x0, y1 - y0);

    let seed = pixel(frame, sx, sy);
    let budget = i64::from(ww) * i64::from(wh) * 3 / 5;

    let at = |x: i32, y: i32| ((y - y0) * ww + (x - x0)) as usize;
    let mut seen = vec![false; (ww * wh) as usize];
    let mut queue = VecDeque::new();
    seen[at(sx, sy)] = true;
    queue.push_back((sx, sy));

    let (mut bx0, mut by0, mut bx1, mut by1) = (sx, sy, sx, sy);
    let mut filled = 0i64;
    while let Some((x, y)) = queue.pop_front() {
        filled += 1;
        if filled > budget {
            return None;
        }
        bx0 = bx0.min(x);
        by0 = by0.min(y);
        bx1 = bx1.max(x);
        by1 = by1.max(y);

        let here = pixel(frame, x, y);
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            if nx < x0 || ny < y0 || nx >= x1 || ny >= y1 {
                continue;
            }
            let i = at(nx, ny);
            if seen[i] {
                continue;
            }
            let there = pixel(frame, nx, ny);
            if dist(here, there) > LOCAL || dist(seed, there) > GLOBAL {
                continue;
            }
            seen[i] = true;
            queue.push_back((nx, ny));
        }
    }

    // Reaching the window edge is only legitimate where the screen itself ends —
    // a control flush against it. Anywhere else it means the grow was still going
    // when it ran out of room.
    let escaped = (bx0 == x0 && x0 > 0)
        || (by0 == y0 && y0 > 0)
        || (bx1 == x1 - 1 && x1 < fw)
        || (by1 == y1 - 1 && y1 < fh);
    if escaped {
        return None;
    }
    Some((bx0, by0, bx1 - bx0 + 1, by1 - by0 + 1))
}

/// The surface `inner` sits on, if any. Tried from just outside each side in
/// turn, because a label's first letter can be hard against the button's border
/// on one side while three others are clear face.
fn enclosing(frame: &Frame, inner: Rect) -> Option<Rect> {
    let (x, y, w, h) = inner;
    const STEP: i32 = 3;
    let seeds = [
        (x - STEP, y + h / 2),
        (x + w - 1 + STEP, y + h / 2),
        (x + w / 2, y - STEP),
        (x + w / 2, y + h - 1 + STEP),
    ];
    seeds
        .into_iter()
        .filter_map(|(sx, sy)| grow(frame, sx, sy))
        .filter(|&outer| contains(outer, inner) && area(outer) >= area(inner) * 2)
        .max_by_key(|&r| area(r))
}

/// The element under `(x, y)`, or `None` when nothing there reads as one.
///
/// A miss is a normal answer, not a failure: the caller keeps whatever the user
/// was doing by hand, and a lasso that guesses when it does not know is worse
/// than one that stays quiet.
pub fn snap_at(frame: &Frame, x: i32, y: i32) -> Option<Rect> {
    let base = grow(frame, x, y);

    // A grow that only caught a letter — or nothing at all, because the cursor
    // was on a glyph whose antialiased edge stopped it dead — is answered by what
    // the letter is printed on.
    let glyphish = base.is_none_or(|(_, _, w, h)| w < GLYPH || h < GLYPH);
    let best = match (base, glyphish) {
        (Some(b), true) => enclosing(frame, b).or(base),
        (Some(_), false) => base,
        (None, _) => None,
    }?;

    let (_, _, w, h) = best;
    (w >= MIN_SIDE && h >= MIN_SIDE).then_some(best)
}

/// The outline a picker shows under the cursor, held across mouse moves.
///
/// Re-growing on every move would also make the outline breathe: [`GLOBAL`] is
/// measured from the seed, so sliding the seed along a gradient shifts which
/// pixels still qualify and the box twitches by a pixel or two under a cursor
/// the user is trying to hold still.
#[derive(Default)]
pub struct Hover {
    shown: Option<Rect>,
    seed: [i32; 3],
}

impl Hover {
    /// Re-aim at `(x, y)`; `true` when the outline changed and the overlay owes
    /// the user a repaint.
    pub fn aim(&mut self, frame: &Frame, x: i32, y: i32) -> bool {
        if self.holds(frame, x, y) {
            return false;
        }
        let next = snap_at(frame, x, y);
        if next.is_some() {
            self.seed = pixel(frame, x, y);
        }
        let changed = next != self.shown;
        self.shown = next;
        changed
    }

    /// Whether what is already outlined still answers for `(x, y)`.
    ///
    /// Both tests are conditions every pixel of the grown set meets, so a point
    /// failing either is provably not part of what is drawn and has earned a
    /// fresh grow. The colour test is the one that matters: a button sitting on
    /// a panel falls inside the panel's box, and without it the panel would stay
    /// outlined however precisely the user aimed at the button.
    fn holds(&self, frame: &Frame, x: i32, y: i32) -> bool {
        let Some((rx, ry, w, h)) = self.shown else { return false };
        let inside = x >= rx && y >= ry && x < rx + w && y < ry + h;
        inside
            && x >= 0
            && y >= 0
            && x < frame.width as i32
            && y < frame.height as i32
            && dist(pixel(frame, x, y), self.seed) <= GLOBAL
    }

    pub fn rect(&self) -> Option<Rect> {
        self.shown
    }

    /// Drop the outline — the caller has started a drag, which supersedes it.
    /// `true` when there was one to drop.
    pub fn clear(&mut self) -> bool {
        self.shown.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame filled with `bg`, into which rectangles can be painted.
    struct Canvas {
        frame: Frame,
    }

    impl Canvas {
        fn new(w: u32, h: u32, bg: [u8; 3]) -> Self {
            let bgr = bg.iter().copied().cycle().take((w * h * 3) as usize).collect();
            Self { frame: Frame { bgr, width: w, height: h } }
        }

        fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 3]) -> &mut Self {
            for yy in y..y + h {
                for xx in x..x + w {
                    let i = (yy as usize * self.frame.width as usize + xx as usize) * 3;
                    self.frame.bgr[i..i + 3].copy_from_slice(&c);
                }
            }
            self
        }

        /// A vertical gradient from `top` to `bottom` — how nearly every game
        /// button is filled, and the case a single tolerance gets wrong.
        fn gradient(&mut self, x: i32, y: i32, w: i32, h: i32, top: u8, bottom: u8) -> &mut Self {
            for yy in y..y + h {
                let t = (yy - y) as i32 * (i32::from(bottom) - i32::from(top)) / (h - 1).max(1);
                let v = (i32::from(top) + t) as u8;
                self.rect(x, yy, w, 1, [v, v, v]);
            }
            self
        }
    }

    #[test]
    fn snaps_to_a_flat_button() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 80, 120, 36, [200, 160, 90]);
        assert_eq!(snap_at(&c.frame, 150, 95), Some((100, 80, 120, 36)));
    }

    #[test]
    fn follows_a_gradient_across_the_whole_control() {
        // 60 levels top to bottom: further than the local tolerance at any one
        // step, so only the two-tolerance rule reaches the far edge.
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.gradient(100, 80, 120, 40, 130, 190);
        assert_eq!(snap_at(&c.frame, 160, 84), Some((100, 80, 120, 40)));
        assert_eq!(snap_at(&c.frame, 160, 118), Some((100, 80, 120, 40)));
    }

    #[test]
    fn a_click_on_the_label_snaps_to_the_button_under_it() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 80, 120, 36, [200, 160, 90]);
        // A letter-sized block of ink in the middle of the face.
        c.rect(150, 90, 10, 16, [30, 25, 20]);
        assert_eq!(snap_at(&c.frame, 154, 96), Some((100, 80, 120, 36)));
    }

    #[test]
    fn the_background_is_not_an_element() {
        // Nothing but background under the cursor: the grow runs out of window.
        let c = Canvas::new(400, 300, [40, 40, 40]);
        assert_eq!(snap_at(&c.frame, 20, 20), None);
    }

    #[test]
    fn a_hairline_is_not_worth_offering() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 80, 200, 3, [200, 160, 90]);
        assert_eq!(snap_at(&c.frame, 160, 81), None);
    }

    #[test]
    fn a_control_flush_against_the_screen_edge_still_snaps() {
        // Touching the frame edge is legitimate; touching the search window's
        // edge anywhere else is not.
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(0, 0, 90, 30, [200, 160, 90]);
        assert_eq!(snap_at(&c.frame, 40, 15), Some((0, 0, 90, 30)));
    }

    #[test]
    fn a_soft_border_does_not_leak_into_the_panel() {
        // Button, then a two-step ramp out to a panel that is itself close in
        // shade. Each step is inside the local tolerance, so only the ceiling on
        // total drift from the seed keeps the grow inside the button.
        let mut c = Canvas::new(400, 300, [150, 150, 150]);
        c.rect(100, 80, 120, 36, [60, 60, 60]);
        c.rect(100, 80, 120, 1, [95, 95, 95]);
        c.rect(100, 79, 120, 1, [130, 130, 130]);
        let (_, y, _, h) = snap_at(&c.frame, 160, 100).expect("button");
        assert!(y >= 80 && y + h <= 116, "leaked out of the button: y={y} h={h}");
    }

    #[test]
    fn a_hover_holds_its_outline_while_the_cursor_stays_on_the_control() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 80, 120, 36, [200, 160, 90]);
        let mut h = Hover::default();

        assert!(h.aim(&c.frame, 150, 95), "the first aim is a change");
        assert_eq!(h.rect(), Some((100, 80, 120, 36)));
        assert!(!h.aim(&c.frame, 190, 105), "moving along the face is not a change");
        assert!(h.aim(&c.frame, 20, 20), "leaving the control drops the outline");
        assert_eq!(h.rect(), None);
    }

    #[test]
    fn a_hover_re_aims_at_a_control_inside_what_it_is_already_showing() {
        // The case the bounding box alone gets wrong: a button on a panel sits
        // inside the panel's box, so only the colour half of the test releases
        // the outline.
        let mut c = Canvas::new(400, 300, [10, 10, 10]);
        c.rect(60, 40, 260, 200, [90, 90, 90]);
        c.rect(120, 100, 100, 40, [200, 160, 90]);
        let mut h = Hover::default();

        h.aim(&c.frame, 80, 60);
        assert_eq!(h.rect(), Some((60, 40, 260, 200)), "the panel");
        assert!(h.aim(&c.frame, 170, 120), "the button is not the panel");
        assert_eq!(h.rect(), Some((120, 100, 100, 40)));
    }

    #[test]
    fn a_cleared_hover_offers_nothing() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 80, 120, 36, [200, 160, 90]);
        let mut h = Hover::default();

        h.aim(&c.frame, 150, 95);
        assert!(h.clear());
        assert_eq!(h.rect(), None);
        assert!(!h.clear(), "clearing twice is not a change");
    }

    #[test]
    fn a_point_outside_the_frame_is_no_element() {
        let c = Canvas::new(64, 64, [40, 40, 40]);
        assert_eq!(snap_at(&c.frame, -1, 10), None);
        assert_eq!(snap_at(&c.frame, 10, 64), None);
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::hardware::capture::Frame;
    use std::time::Instant;

    #[test]
    #[ignore = "scratch"]
    fn worst_case_hover() {
        // A flat desktop: every grow runs the full budget before answering None.
        let f = Frame { bgr: vec![40u8; 2560 * 1440 * 3], width: 2560, height: 1440 };
        let t = Instant::now();
        for i in 0..20 {
            assert_eq!(snap_at(&f, 1200 + i, 700), None);
        }
        println!("BACKGROUND: {:?} per hover", t.elapsed() / 20);

        // A button-sized control: the common case.
        let mut c = Frame { bgr: vec![40u8; 2560 * 1440 * 3], width: 2560, height: 1440 };
        for y in 700..760 {
            for x in 1200..1400 {
                let i = (y * 2560 + x) * 3;
                c.bgr[i..i + 3].copy_from_slice(&[200, 160, 90]);
            }
        }
        let t = Instant::now();
        for i in 0..20 {
            assert!(snap_at(&c, 1250 + i, 730).is_some());
        }
        println!("BUTTON: {:?} per hover", t.elapsed() / 20);
    }
}
