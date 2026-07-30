//! `cv2.Canny(src, 50, 150)`, transcribed: 3x3 Sobel, L1 gradient magnitude,
//! directional non-maximum suppression, hysteresis.
//!
//! Edges are the detector's second matching tier and the skeleton `_auto_mask`
//! flood-fills against. They matter because an edge image throws away absolute
//! brightness entirely: a button rendered over grass and the same button over
//! sand correlate poorly as pixels and well as outlines.

use super::image::Gray;

/// OpenCV's fixed-point tangent comparison: `round(tan(22.5°) * 2^15)`.
const CANNY_SHIFT: i32 = 15;
const TG22: i32 = 13573;

/// The 8 neighbours, for hysteresis propagation.
const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

pub fn canny(src: &Gray, low: i32, high: i32) -> Gray {
    let (low, high) = if low > high { (high, low) } else { (low, high) };
    if src.w < 2 || src.h < 2 {
        return Gray::new(src.w, src.h);
    }
    let (w, h) = (src.w, src.h);
    let (dx, dy) = sobel3(src);

    // L1 magnitude, OpenCV's default (`L2gradient=False`).
    let mag: Vec<i32> = dx
        .iter()
        .zip(&dy)
        .map(|(&x, &y)| x.abs() + y.abs())
        .collect();
    let magnitude = |x: i32, y: i32| -> i32 {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            0
        } else {
            mag[y as usize * w + x as usize]
        }
    };

    // 0 = survived suppression but undecided, 1 = suppressed, 2 = edge.
    let mut map = vec![1u8; w * h];
    let mut stack: Vec<usize> = Vec::new();

    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = y as usize * w + x as usize;
            let m = mag[i];
            if m <= low {
                continue;
            }
            let xs = dx[i];
            let ys = dy[i];
            let ax = xs.abs();
            let ay = ys.abs() << CANNY_SHIFT;
            let tg22x = ax * TG22;

            // Which of the four gradient octant pairs the edge normal falls in
            // decides which two neighbours the magnitude must beat.
            let is_max = if ay < tg22x {
                m > magnitude(x - 1, y) && m >= magnitude(x + 1, y)
            } else {
                let tg67x = tg22x + (ax << (CANNY_SHIFT + 1));
                if ay > tg67x {
                    m > magnitude(x, y - 1) && m >= magnitude(x, y + 1)
                } else {
                    let s = if (xs ^ ys) < 0 { -1 } else { 1 };
                    m > magnitude(x - s, y - 1) && m > magnitude(x + s, y + 1)
                }
            };
            if !is_max {
                continue;
            }
            if m > high {
                map[i] = 2;
                stack.push(i);
            } else {
                map[i] = 0;
            }
        }
    }

    // Hysteresis: a weak survivor is kept only if it is 8-connected to a strong
    // one, transitively.
    while let Some(i) = stack.pop() {
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        for (dx, dy) in NEIGHBOURS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let ni = ny as usize * w + nx as usize;
            if map[ni] == 0 {
                map[ni] = 2;
                stack.push(ni);
            }
        }
    }

    Gray::from_vec(
        w,
        h,
        map.into_iter()
            .map(|v| if v == 2 { 255 } else { 0 })
            .collect(),
    )
}

/// The two 3x3 Sobel derivatives, `BORDER_REPLICATE` at the edges, the same
/// pair `cv2.Canny` computes internally.
fn sobel3(src: &Gray) -> (Vec<i32>, Vec<i32>) {
    let (w, h) = (src.w, src.h);
    let mut dx = vec![0i32; w * h];
    let mut dy = vec![0i32; w * h];
    let at = |x: i32, y: i32| -> i32 {
        let cx = x.clamp(0, w as i32 - 1) as usize;
        let cy = y.clamp(0, h as i32 - 1) as usize;
        i32::from(src.at(cx, cy))
    };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let tl = at(x - 1, y - 1);
            let tc = at(x, y - 1);
            let tr = at(x + 1, y - 1);
            let ml = at(x - 1, y);
            let mr = at(x + 1, y);
            let bl = at(x - 1, y + 1);
            let bc = at(x, y + 1);
            let br = at(x + 1, y + 1);
            let i = y as usize * w + x as usize;
            dx[i] = (tr + 2 * mr + br) - (tl + 2 * ml + bl);
            dy[i] = (bl + 2 * bc + br) - (tl + 2 * tc + tr);
        }
    }
    (dx, dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_image_has_no_edges() {
        let src = Gray::from_vec(20, 20, vec![120; 400]);
        assert_eq!(canny(&src, 50, 150).count_nonzero(), 0);
    }

    #[test]
    fn a_vertical_step_produces_one_vertical_line() {
        let mut src = Gray::new(21, 11);
        for y in 0..11 {
            for x in 10..21 {
                src.set(x, y, 255);
            }
        }
        let e = canny(&src, 50, 150);
        // The response sits on the step, one column wide, and spans the height.
        for y in 0..11 {
            let lit: Vec<usize> = (0..21).filter(|&x| e.at(x, y) != 0).collect();
            assert_eq!(lit.len(), 1, "row {y} lit {lit:?}");
            assert!(lit[0] == 9 || lit[0] == 10, "row {y} lit at {}", lit[0]);
        }
    }

    #[test]
    fn a_gradient_too_weak_for_the_low_threshold_is_dropped() {
        // A one-level step: |dx| peaks at 4, far under a low threshold of 50.
        let mut src = Gray::new(12, 6);
        for y in 0..6 {
            for x in 6..12 {
                src.set(x, y, 101);
            }
        }
        for y in 0..6 {
            for x in 0..6 {
                src.set(x, y, 100);
            }
        }
        assert_eq!(canny(&src, 50, 150).count_nonzero(), 0);
    }

    #[test]
    fn hysteresis_keeps_a_weak_run_attached_to_a_strong_one() {
        // Left half of the step is a full 0→255 jump (strong), the right half a
        // 0→90 jump (over `low`, under `high`). The weak part survives because
        // it is connected to the strong part.
        let mut src = Gray::new(24, 12);
        for y in 0..12 {
            let level = if y < 6 { 255 } else { 90 };
            for x in 12..24 {
                src.set(x, y, level);
            }
        }
        let e = canny(&src, 50, 150);
        let bottom_lit = (0..24).any(|x| e.at(x, 9) != 0);
        assert!(bottom_lit, "the weak half of the edge was dropped");
    }

    #[test]
    fn an_image_too_small_to_have_a_neighbourhood_comes_back_blank() {
        let src = Gray::from_vec(1, 1, vec![255]);
        assert_eq!(canny(&src, 50, 150).data, vec![0]);
    }
}
