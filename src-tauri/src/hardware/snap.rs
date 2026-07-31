//! Snap-to-element: the "smart lasso" the screen pickers offer in place of a
//! hand-drawn box.
//!
//! Point at a button and this returns the button's rectangle. Drawing a tight
//! box by hand is the fiddliest part of setting up a template or a surgical
//! click: a few pixels of the surrounding panel dragged in with it is enough to
//! stop the template matching once the panel scrolls, and a few pixels short
//! clips the glyph the match keys on.
//!
//! Three methods answer, because no one of them can.
//!
//! The first is a region grow from the pixel under the cursor, with two
//! tolerances rather than one. A pixel joins if it is close to the neighbour it
//! was reached from (so a gradient-filled control is followed all the way across)
//! *and* still within a wider distance of the seed (so the grow cannot drift
//! through a soft edge into the panel behind). This is the rule a magic-wand
//! tool uses, and it is what makes the difference on game UI, which is nearly all
//! gradients and soft borders.
//!
//! What it cannot do is see anything that is not one flat-ish surface, and a
//! great deal of game UI is not: a textured or patterned button face stops the
//! grow at the first pixel of pattern, and an icon with its label beside it is
//! two shapes with a hole between them that no fill will ever join. Both come
//! back as a speck or as nothing, which is the "smart lasso misses things"
//! everyone runs into.
//!
//! So the second method looks for the element's *outline* instead of its fill.
//! Every pair of touching pixels that differ sharply is marked, short gaps in the
//! marks are closed along each axis (run-length smoothing, the same step that
//! turns scattered letters into a paragraph), and what is left is grouped into
//! connected shapes. A patterned face is dense with marks and comes back as one
//! shape; an icon and the words beside it are bridged by the gap-closing and come
//! back as one shape; two controls with real air between them stay two.
//!
//! What *that* cannot do is separate things drawn hard against each other. An
//! inventory grid is the case: the gutter between two slots is a few pixels, the
//! gap-closing bridges it, and a whole row of slots comes back as one shape,
//! usually so large it runs off the window and is thrown away, which is the "it
//! found nothing" everyone else runs into.
//!
//! So the third method looks for the element's *sides*. It walks out from the
//! cursor along its row and its column until it crosses a difference that holds
//! for a stretch of pixels rather than one, which is the difference between the
//! side of something and a speck of pattern. Four walks give four sides, and a
//! slot in a grid has all four whether or not its neighbours are a pixel away.
//!
//! None of the three is trusted over the others by name. Each offers a box, and
//! the boxes are ranked by how much of their perimeter sits on a real
//! discontinuity. A grow that leaked through a soft corner, or two controls the
//! gap-closing ran together, have edges down only part of their sides, and the
//! box a person would have drawn by hand has all of them. The exception is
//! speed: a grow that comes back control-sized and fully bordered is returned on
//! the spot, because nothing can beat a per-pixel fill that already agrees with
//! the edges, and that is the case the cursor is in most of the time.
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

/// How far apart two touching pixels must be to count as an outline. Low enough
/// to catch the soft borders game UI is drawn with, high enough that a gradient's
/// step from one row to the next is not an outline in its own right.
const EDGE: i32 = 18;

/// Blank runs shorter than these are closed before outlines are grouped into
/// shapes: the distance across which an icon still belongs to the words beside
/// it. Wider horizontally than vertically because that is how UI is laid out:
/// things that belong together sit side by side more often than stacked, and
/// separate rows of controls are usually closer than separate columns.
const H_GAP: i32 = 14;
const V_GAP: i32 = 10;

/// The window the outline pass works in. Smaller than the grow's, because it
/// looks at every pixel in it several times and a control that fills more than
/// this is not what the lasso is for.
const INK_W: i32 = 640;
const INK_H: i32 = 400;

/// The window the side-walk works in. Much the largest of the three, because it
/// only ever reads a line of pixels at a time where the others read areas, and
/// because reaching the things they cannot is the whole point of it: a panel or
/// a toolbar wider than the grow's window is exactly what comes back as nothing
/// today.
const RULE_W: i32 = 1600;
const RULE_H: i32 = 1000;

/// How far either side of the cursor a difference must hold to count as the side
/// of something rather than a speck of pattern. Half of [`MIN_SIDE`], so the
/// smallest control still worth offering can supply the whole run itself.
const RUN: i32 = 7;

/// What share of a run has to be a difference, as a percentage. Not all of it:
/// antialiasing softens a border to nothing for a row here and there, and a
/// corner radius takes the last pixels off both ends of every side.
const RUN_HIT: i32 = 70;

/// Perimeter a grow must have on real edges before it is returned without
/// consulting anything else. Under 1.0 because rounded corners are not edges.
const SURE: f32 = 0.85;

/// How far a ruled box's sides must stand out from its own face. A patterned
/// surface has a "side" every few pixels and the walk stops on the nearest one;
/// what tells that apart from a control is that a control's face is quiet.
const CONTRAST: f32 = 0.25;

/// Per-channel distance: the same max-abs metric the guard colour match uses,
/// which is what makes a snapped region and a colour guard agree about what
/// counts as "the same shade".
fn dist(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0])
        .abs()
        .max((a[1] - b[1]).abs())
        .max((a[2] - b[2]).abs())
}

fn pixel(frame: &Frame, x: i32, y: i32) -> [i32; 3] {
    let i = (y as usize * frame.width as usize + x as usize) * 3;
    [
        i32::from(frame.bgr[i]),
        i32::from(frame.bgr[i + 1]),
        i32::from(frame.bgr[i + 2]),
    ]
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

    // Reaching the window edge is only legitimate where the screen itself ends:
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

fn holds_point((rx, ry, w, h): Rect, x: i32, y: i32) -> bool {
    x >= rx && y >= ry && x < rx + w && y < ry + h
}

/// Big enough to read as a control rather than as a letter of one's label.
fn is_control((_, _, w, h): Rect) -> bool {
    w >= GLYPH && h >= GLYPH
}

/// The share of `rect`'s perimeter that sits on a real discontinuity, 0 to 1.
///
/// This is the one measure every method's answer can be held to, whatever it was
/// derived from, and it says the thing a person means by "that's the button":
/// the box changes colour all the way round. Where the frame itself ends counts,
/// because a control flush against the screen edge has nothing beyond it to
/// differ from.
fn border_support(frame: &Frame, (x, y, w, h): Rect) -> f32 {
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    let mut hits = 0i32;
    let mut total = 0i32;
    let mut side = |inside: (i32, i32), outside: (i32, i32)| {
        total += 1;
        if outside.0 < 0 || outside.1 < 0 || outside.0 >= fw || outside.1 >= fh {
            hits += 1;
        } else if dist(
            pixel(frame, inside.0, inside.1),
            pixel(frame, outside.0, outside.1),
        ) > EDGE
        {
            hits += 1;
        }
    };
    for yy in y.max(0)..(y + h).min(fh) {
        side((x, yy), (x - 1, yy));
        side((x + w - 1, yy), (x + w, yy));
    }
    for xx in x.max(0)..(x + w).min(fw) {
        side((xx, y), (xx, y - 1));
        side((xx, y + h - 1), (xx, y + h));
    }
    if total == 0 {
        return 0.0;
    }
    hits as f32 / total as f32
}

/// The share of pixels inside `rect` that sit on a discontinuity: how busy the
/// face is. Sampled on a coarse grid, since this only has to tell a flat face
/// from a patterned one, and on a stride coprime with the small tile sizes UI
/// patterns use so it cannot land on the same phase every time and see nothing.
fn ink_density(frame: &Frame, (x, y, w, h): Rect) -> f32 {
    const STEP: i32 = 3;
    let (x0, y0) = (x + 1, y + 1);
    let (x1, y1) = (x + w - 1, y + h - 1);
    let (mut hits, mut total) = (0i32, 0i32);
    let mut yy = y0;
    while yy < y1 {
        let mut xx = x0;
        while xx < x1 {
            total += 1;
            let here = pixel(frame, xx, yy);
            if dist(here, pixel(frame, xx - 1, yy)) > EDGE
                || dist(here, pixel(frame, xx, yy - 1)) > EDGE
            {
                hits += 1;
            }
            xx += STEP;
        }
        yy += STEP;
    }
    if total == 0 {
        return 0.0;
    }
    hits as f32 / total as f32
}

/// Whether a side runs between columns `c - 1` and `c`, judged over the rows
/// around `sy` rather than at the cursor's own row. One pixel of difference is
/// texture; a column of it is the edge of something.
fn vertical_side(frame: &Frame, c: i32, sy: i32) -> bool {
    let fh = frame.height as i32;
    let (y0, y1) = ((sy - RUN).max(0), (sy + RUN + 1).min(fh));
    let hits = (y0..y1)
        .filter(|&y| dist(pixel(frame, c - 1, y), pixel(frame, c, y)) > EDGE)
        .count() as i32;
    hits * 100 >= (y1 - y0) * RUN_HIT
}

/// [`vertical_side`] turned on its side.
fn horizontal_side(frame: &Frame, r: i32, sx: i32) -> bool {
    let fw = frame.width as i32;
    let (x0, x1) = ((sx - RUN).max(0), (sx + RUN + 1).min(fw));
    let hits = (x0..x1)
        .filter(|&x| dist(pixel(frame, x, r - 1), pixel(frame, x, r)) > EDGE)
        .count() as i32;
    hits * 100 >= (x1 - x0) * RUN_HIT
}

/// Walk out from the cursor in all four directions and return the box the four
/// sides make.
///
/// `None` when any direction ran to the end of the search window without
/// crossing a side: whatever the cursor is on carries on past what we can see,
/// and a box that stops where the window does is a box the user did not pick.
fn ruled(frame: &Frame, sx: i32, sy: i32) -> Option<Rect> {
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    let wx0 = (sx - RULE_W / 2).max(0);
    let wy0 = (sy - RULE_H / 2).max(0);
    let wx1 = (sx + RULE_W / 2).min(fw);
    let wy1 = (sy + RULE_H / 2).min(fh);

    // A side found at `c` on the way out is the first column of the element on
    // the near sides and the first column past it on the far ones, so left and
    // top land on it and right and bottom stop short of it.
    let left = match (wx0 + 1..=sx).rev().find(|&c| vertical_side(frame, c, sy)) {
        Some(c) => c,
        None if wx0 == 0 => 0,
        None => return None,
    };
    let right = match (sx + 1..wx1).find(|&c| vertical_side(frame, c, sy)) {
        Some(c) => c,
        None if wx1 == fw => fw,
        None => return None,
    };
    let top = match (wy0 + 1..=sy)
        .rev()
        .find(|&r| horizontal_side(frame, r, sx))
    {
        Some(r) => r,
        None if wy0 == 0 => 0,
        None => return None,
    };
    let bottom = match (sy + 1..wy1).find(|&r| horizontal_side(frame, r, sx)) {
        Some(r) => r,
        None if wy1 == fh => fh,
        None => return None,
    };

    let rect = (left, top, right - left, bottom - top);
    if rect.2 < MIN_SIDE || rect.3 < MIN_SIDE {
        return None;
    }
    // Four walks that all ran to the screen's own edges have found the screen.
    // The same ceiling the other two methods keep, and for the same reason: past
    // it the answer stops being an element the user pointed at.
    if area(rect) > i64::from(fw) * i64::from(fh) * 3 / 5 {
        return None;
    }
    let support = border_support(frame, rect);
    if support < 0.7 || support < ink_density(frame, rect) + CONTRAST {
        return None;
    }
    Some(rect)
}

/// Bridge the blank runs shorter than `gap` along one axis of `ink`, writing the
/// bridges into `out`.
///
/// Both halves read the same original marks, so calling this once per axis and
/// keeping the union closes gaps *either* way round rather than letting the first
/// pass feed the second, which would run a bridge into a bridge and walk clean
/// across the screen.
fn close_runs(out: &mut [bool], ink: &[bool], ww: i32, wh: i32, gap: i32, horizontal: bool) {
    let (lines, len) = if horizontal { (wh, ww) } else { (ww, wh) };
    for line in 0..lines {
        let at = |i: i32| {
            if horizontal {
                (line * ww + i) as usize
            } else {
                (i * ww + line) as usize
            }
        };
        let mut last: Option<i32> = None;
        for i in 0..len {
            if !ink[at(i)] {
                continue;
            }
            if let Some(prev) = last {
                let blank = i - prev - 1;
                if blank > 0 && blank <= gap {
                    for j in prev + 1..i {
                        out[at(j)] = true;
                    }
                }
            }
            last = Some(i);
        }
    }
}

/// Every outlined shape whose box holds `(sx, sy)`, smallest first.
///
/// Membership is by *box*, not by pixel: the cursor is usually on the blank face
/// of a control, which is the one part of it that carries no outline. The shape
/// is the ring around the face, and the face is inside the ring's box.
///
/// A shape is dropped if it ran into the edge of the window (it continues past
/// anything we can see, so its box means nothing) or fills most of it (a screen
/// busy enough to smear into one shape has told us nothing about what is under
/// the cursor).
fn ink_candidates(frame: &Frame, sx: i32, sy: i32) -> Vec<Rect> {
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    let x0 = (sx - INK_W / 2).max(0);
    let y0 = (sy - INK_H / 2).max(0);
    let x1 = (sx + INK_W / 2).min(fw);
    let y1 = (sy + INK_H / 2).min(fh);
    let (ww, wh) = (x1 - x0, y1 - y0);
    if ww < 3 || wh < 3 {
        return Vec::new();
    }
    let at = |x: i32, y: i32| ((y - y0) * ww + (x - x0)) as usize;

    // Both sides of a discontinuity are marked, so a shape's outline sits a pixel
    // outside the shape all the way round. That pixel is taken back off at the
    // end, which is what lets this agree with the grow to the pixel on a control
    // both of them can see.
    let mut ink = vec![false; (ww * wh) as usize];
    for y in y0..y1 {
        for x in x0..x1 {
            let here = pixel(frame, x, y);
            if x + 1 < x1 && dist(here, pixel(frame, x + 1, y)) > EDGE {
                ink[at(x, y)] = true;
                ink[at(x + 1, y)] = true;
            }
            if y + 1 < y1 && dist(here, pixel(frame, x, y + 1)) > EDGE {
                ink[at(x, y)] = true;
                ink[at(x, y + 1)] = true;
            }
        }
    }

    let mut closed = ink.clone();
    close_runs(&mut closed, &ink, ww, wh, H_GAP, true);
    close_runs(&mut closed, &ink, ww, wh, V_GAP, false);

    let cap = i64::from(ww) * i64::from(wh) * 3 / 5;
    let mut seen = vec![false; (ww * wh) as usize];
    let mut queue = VecDeque::new();
    let mut found = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            if !closed[at(x, y)] || seen[at(x, y)] {
                continue;
            }
            seen[at(x, y)] = true;
            queue.push_back((x, y));
            let (mut bx0, mut by0, mut bx1, mut by1) = (x, y, x, y);
            while let Some((cx, cy)) = queue.pop_front() {
                bx0 = bx0.min(cx);
                by0 = by0.min(cy);
                bx1 = bx1.max(cx);
                by1 = by1.max(cy);
                // Eight-connected: a diagonal step is how a rounded corner or a
                // one-pixel outline gets round the bend without a gap.
                for ny in cy - 1..=cy + 1 {
                    for nx in cx - 1..=cx + 1 {
                        if nx < x0 || ny < y0 || nx >= x1 || ny >= y1 {
                            continue;
                        }
                        if !closed[at(nx, ny)] || seen[at(nx, ny)] {
                            continue;
                        }
                        seen[at(nx, ny)] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            let escaped = (bx0 == x0 && x0 > 0)
                || (by0 == y0 && y0 > 0)
                || (bx1 == x1 - 1 && x1 < fw)
                || (by1 == y1 - 1 && y1 < fh);
            // Back off the outline's own pixel on each side.
            let rect = (
                bx0 + 1,
                by0 + 1,
                (bx1 - bx0 - 1).max(0),
                (by1 - by0 - 1).max(0),
            );
            if !escaped && area(rect) <= cap && holds_point(rect, sx, sy) {
                found.push(rect);
            }
        }
    }

    found.sort_by_key(|&r| area(r));
    found
}

/// The element under `(x, y)`, or `None` when nothing there reads as one.
///
/// A miss is a normal answer, not a failure: the caller keeps whatever the user
/// was doing by hand, and a lasso that guesses when it does not know is worse
/// than one that stays quiet.
pub fn snap_at(frame: &Frame, x: i32, y: i32) -> Option<Rect> {
    let (fw, fh) = (frame.width as i32, frame.height as i32);
    if x < 0 || y < 0 || x >= fw || y >= fh {
        return None;
    }

    let base = grow(frame, x, y);

    // A grow that came back with a whole control, bordered the whole way round,
    // is the answer and the exact one; nothing else can improve on a per-pixel
    // fill that already agrees with the edges. Everything below is for what the
    // grow is unsure of, which keeps the common case costing what it always did.
    if let Some(sure) = base.filter(|&r| is_control(r)) {
        if border_support(frame, sure) >= SURE {
            return Some(sure);
        }
    }

    let mut pool = ink_candidates(frame, x, y);
    pool.extend(base);
    pool.extend(base.and_then(|b| enclosing(frame, b)));
    pool.extend(ruled(frame, x, y));
    pool.retain(|&(_, _, w, h)| w >= MIN_SIDE && h >= MIN_SIDE);
    pool.sort_unstable();
    pool.dedup();
    pool.sort_by_key(|&r| area(r));

    // Of the boxes that read as a control, the one whose sides are most nearly
    // all real edges, and the tightest of those where several are: a button and
    // the panel behind it are both fully bordered, and the button is the one
    // under the cursor. Failing any, the largest thing big enough to offer at
    // all: a glyph's surroundings are what the user was pointing at when the
    // glyph is all there is.
    pool.iter()
        .copied()
        .filter(|&r| is_control(r))
        .map(|r| (r, border_support(frame, r)))
        .reduce(|best, next| if next.1 > best.1 { next } else { best })
        .map(|(r, _)| r)
        .or_else(|| pool.last().copied())
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
        let Some((rx, ry, w, h)) = self.shown else {
            return false;
        };
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

    /// Drop the outline: the caller has started a drag, which supersedes it.
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
            let bgr = bg
                .iter()
                .copied()
                .cycle()
                .take((w * h * 3) as usize)
                .collect();
            Self {
                frame: Frame {
                    bgr,
                    width: w,
                    height: h,
                },
            }
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

        /// A two-shade check in 2x2 tiles, a stand-in for the patterned and
        /// textured faces game UI is full of, and the thing a region grow cannot
        /// cross: same-shade tiles touch only at their corners, so a four-way
        /// fill gets exactly one of them.
        fn checker(&mut self, x: i32, y: i32, w: i32, h: i32, a: [u8; 3], b: [u8; 3]) -> &mut Self {
            for yy in y..y + h {
                for xx in x..x + w {
                    let odd = ((xx - x) / 2 + (yy - y) / 2) % 2 == 1;
                    self.rect(xx, yy, 1, 1, if odd { b } else { a });
                }
            }
            self
        }

        /// A vertical gradient from `top` to `bottom`, how nearly every game
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

    /// The case the region grow cannot answer at all: a face with a pattern on
    /// it. Every step across the pattern is further than the local tolerance, so
    /// the fill gets one tile and the lasso used to offer nothing.
    #[test]
    fn snaps_to_a_patterned_button() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.checker(150, 120, 100, 40, [120, 120, 120], [170, 170, 170]);
        assert_eq!(snap_at(&c.frame, 200, 140), Some((150, 120, 100, 40)));
    }

    /// An icon and its label, with nothing drawn behind them. There is no surface
    /// to fall back to, so the grow's answer is the icon on its own; the gap
    /// closing joins the two into the control they are.
    #[test]
    fn an_icon_and_its_label_are_one_element() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 100, 16, 16, [200, 160, 90]);
        for i in 0..3 {
            c.rect(124 + i * 8, 102, 5, 12, [210, 210, 210]);
        }
        assert_eq!(snap_at(&c.frame, 106, 108), Some((100, 100, 45, 16)));
    }

    /// And pointing at the air between them answers with the same control: the
    /// gap is inside what the two of them make up, which is the whole reason to
    /// group by box rather than by which pixels the cursor is actually on.
    #[test]
    fn the_gap_inside_a_group_still_snaps_to_the_group() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 100, 16, 16, [200, 160, 90]);
        for i in 0..3 {
            c.rect(124 + i * 8, 102, 5, 12, [210, 210, 210]);
        }
        assert_eq!(snap_at(&c.frame, 120, 108), Some((100, 100, 45, 16)));
    }

    /// Two controls with real air between them stay two, which is the limit the
    /// gap closing has to respect to be worth having.
    #[test]
    fn neighbouring_controls_are_not_merged() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.checker(100, 100, 60, 30, [120, 120, 120], [170, 170, 170]);
        c.checker(190, 100, 60, 30, [120, 120, 120], [170, 170, 170]);
        assert_eq!(snap_at(&c.frame, 130, 115), Some((100, 100, 60, 30)));
        assert_eq!(snap_at(&c.frame, 220, 115), Some((190, 100, 60, 30)));
    }

    /// A control shaded harder than the ceiling on how far the fill may drift
    /// from the seed. The fill stops partway down it and hands back a box cut
    /// across the middle of the button, which used to be returned as-is,
    /// because it was control-sized and nothing checked whether its sides were
    /// on anything. They are not: the cut edge runs through flat gradient.
    #[test]
    fn a_control_shaded_past_the_fill_ceiling_still_comes_back_whole() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.gradient(100, 80, 120, 60, 80, 230);
        assert_eq!(snap_at(&c.frame, 160, 86), Some((100, 80, 120, 60)));
    }

    /// A panel wider than the fill's own search window, with a quiet face. The
    /// fill runs out of window, and the outline pass sees no outline because the
    /// edges of the thing are outside *its* window too. Only walking out along
    /// the cursor's own row and column reaches the sides.
    #[test]
    fn a_panel_larger_than_the_other_two_windows_still_snaps() {
        let mut c = Canvas::new(1400, 900, [20, 20, 20]);
        c.rect(100, 100, 900, 600, [90, 90, 90]);
        assert_eq!(
            grow(&c.frame, 550, 400),
            None,
            "the fill was supposed to run out of window"
        );
        assert_eq!(snap_at(&c.frame, 550, 400), Some((100, 100, 900, 600)));
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
        assert!(
            y >= 80 && y + h <= 116,
            "leaked out of the button: y={y} h={h}"
        );
    }

    #[test]
    fn a_hover_holds_its_outline_while_the_cursor_stays_on_the_control() {
        let mut c = Canvas::new(400, 300, [40, 40, 40]);
        c.rect(100, 80, 120, 36, [200, 160, 90]);
        let mut h = Hover::default();

        assert!(h.aim(&c.frame, 150, 95), "the first aim is a change");
        assert_eq!(h.rect(), Some((100, 80, 120, 36)));
        assert!(
            !h.aim(&c.frame, 190, 105),
            "moving along the face is not a change"
        );
        assert!(
            h.aim(&c.frame, 20, 20),
            "leaving the control drops the outline"
        );
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
    #[ignore = "timing benchmark, run by hand: prints, asserts nothing about duration"]
    fn worst_case_hover() {
        // A flat desktop: every grow runs the full budget before answering None.
        let f = Frame {
            bgr: vec![40u8; 2560 * 1440 * 3],
            width: 2560,
            height: 1440,
        };
        let t = Instant::now();
        for i in 0..20 {
            assert_eq!(snap_at(&f, 1200 + i, 700), None);
        }
        println!("BACKGROUND: {:?} per hover", t.elapsed() / 20);

        // A button-sized control: the common case.
        let mut c = Frame {
            bgr: vec![40u8; 2560 * 1440 * 3],
            width: 2560,
            height: 1440,
        };
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
