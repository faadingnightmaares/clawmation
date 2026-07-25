//! `cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8))`, transcribed.
//!
//! Contrast-limited adaptive histogram equalisation is what makes template
//! matching survive a brightness shift: both the template and the search area go
//! through it before they are correlated, so a scene that got darker still
//! correlates against a template captured when it was bright. The tile LUTs and
//! the bilinear blend between them are OpenCV's, including its quirk of padding
//! by a whole tile when a dimension already divides evenly.

use super::image::{cv_floor, saturate_u8, Gray};

const HIST_SIZE: usize = 256;

/// The equaliser, holding only its parameters — OpenCV's `CLAHE` object caches
/// nothing between calls either.
pub struct Clahe {
    clip_limit: f64,
    tiles_x: usize,
    tiles_y: usize,
}

impl Clahe {
    pub fn new(clip_limit: f64, tiles_x: usize, tiles_y: usize) -> Self {
        Self { clip_limit, tiles_x, tiles_y }
    }

    /// The detector's settings: `clipLimit=2.0`, `tileGridSize=(8, 8)`.
    pub fn detector_default() -> Self {
        Self::new(2.0, 8, 8)
    }

    pub fn apply(&self, src: &Gray) -> Gray {
        if src.is_empty() {
            return src.clone();
        }
        // OpenCV builds the LUTs off a padded copy whenever *either* dimension
        // fails to divide evenly — and then pads a whole extra tile on the axis
        // that did divide. Reproduced rather than corrected: the LUTs, and so
        // every equalised pixel, depend on it.
        let (lut_src, tile_w, tile_h) = if src.w % self.tiles_x == 0 && src.h % self.tiles_y == 0 {
            (src.clone(), src.w / self.tiles_x, src.h / self.tiles_y)
        } else {
            let padded = reflect_101(
                src,
                self.tiles_x - (src.w % self.tiles_x),
                self.tiles_y - (src.h % self.tiles_y),
            );
            let (tw, th) = (padded.w / self.tiles_x, padded.h / self.tiles_y);
            (padded, tw, th)
        };
        if tile_w == 0 || tile_h == 0 {
            return src.clone();
        }

        let tile_total = tile_w * tile_h;
        let lut_scale = (HIST_SIZE - 1) as f32 / tile_total as f32;
        let clip = if self.clip_limit > 0.0 {
            ((self.clip_limit * tile_total as f64 / HIST_SIZE as f64) as i32).max(1)
        } else {
            0
        };

        let mut luts = vec![0u8; self.tiles_x * self.tiles_y * HIST_SIZE];
        for ty in 0..self.tiles_y {
            for tx in 0..self.tiles_x {
                let mut hist = [0i32; HIST_SIZE];
                for y in ty * tile_h..(ty + 1) * tile_h {
                    for x in tx * tile_w..(tx + 1) * tile_w {
                        hist[lut_src.at(x, y) as usize] += 1;
                    }
                }
                if clip > 0 {
                    clip_histogram(&mut hist, clip);
                }
                let base = (ty * self.tiles_x + tx) * HIST_SIZE;
                let mut sum = 0i32;
                for i in 0..HIST_SIZE {
                    sum += hist[i];
                    luts[base + i] = saturate_u8(sum as f32 * lut_scale);
                }
            }
        }

        self.interpolate(src, &luts, tile_w, tile_h)
    }

    /// Blend the four surrounding tile LUTs per pixel — OpenCV's
    /// `CLAHE_Interpolation_Body`, half-pixel offsets and edge clamping included.
    fn interpolate(&self, src: &Gray, luts: &[u8], tile_w: usize, tile_h: usize) -> Gray {
        let inv_tw = 1.0 / tile_w as f32;
        let inv_th = 1.0 / tile_h as f32;
        let mut out = Gray::new(src.w, src.h);

        for y in 0..src.h {
            let tyf = y as f32 * inv_th - 0.5;
            let ty1 = cv_floor(tyf);
            let ya = tyf - ty1 as f32;
            let ty2 = (ty1 + 1).min(self.tiles_y as i32 - 1).max(0) as usize;
            let ty1 = ty1.clamp(0, self.tiles_y as i32 - 1) as usize;
            let plane1 = ty1 * self.tiles_x * HIST_SIZE;
            let plane2 = ty2 * self.tiles_x * HIST_SIZE;

            for x in 0..src.w {
                let txf = x as f32 * inv_tw - 0.5;
                let tx1 = cv_floor(txf);
                let xa = txf - tx1 as f32;
                let tx2 = (tx1 + 1).min(self.tiles_x as i32 - 1).max(0) as usize;
                let tx1 = tx1.clamp(0, self.tiles_x as i32 - 1) as usize;

                let v = src.at(x, y) as usize;
                let a = f32::from(luts[plane1 + tx1 * HIST_SIZE + v]);
                let b = f32::from(luts[plane1 + tx2 * HIST_SIZE + v]);
                let c = f32::from(luts[plane2 + tx1 * HIST_SIZE + v]);
                let d = f32::from(luts[plane2 + tx2 * HIST_SIZE + v]);

                let res = a * ((1.0 - xa) * (1.0 - ya))
                    + b * (xa * (1.0 - ya))
                    + c * ((1.0 - xa) * ya)
                    + d * (xa * ya);
                out.set(x, y, saturate_u8(res));
            }
        }
        out
    }
}

/// Cap every bin at `clip` and hand the excess back out evenly, the residual
/// spread at a fixed stride — OpenCV redistributes rather than discards, which
/// is what keeps the LUT monotonic after clipping.
fn clip_histogram(hist: &mut [i32; HIST_SIZE], clip: i32) {
    let mut clipped = 0i32;
    for bin in hist.iter_mut() {
        if *bin > clip {
            clipped += *bin - clip;
            *bin = clip;
        }
    }
    if clipped == 0 {
        return;
    }
    let batch = clipped / HIST_SIZE as i32;
    let mut residual = clipped - batch * HIST_SIZE as i32;
    for bin in hist.iter_mut() {
        *bin += batch;
    }
    if residual > 0 {
        let step = (HIST_SIZE as i32 / residual).max(1) as usize;
        let mut i = 0;
        while i < HIST_SIZE && residual > 0 {
            hist[i] += 1;
            residual -= 1;
            i += step;
        }
    }
}

/// `cv2.copyMakeBorder(..., BORDER_REFLECT_101)` on the right and bottom edges.
fn reflect_101(src: &Gray, right: usize, bottom: usize) -> Gray {
    let (w, h) = (src.w + right, src.h + bottom);
    let mut out = Gray::new(w, h);
    for y in 0..h {
        let sy = reflect_index(y as i32, src.h as i32);
        for x in 0..w {
            let sx = reflect_index(x as i32, src.w as i32);
            out.set(x, y, src.at(sx, sy));
        }
    }
    out
}

/// `cv::borderInterpolate` for `BORDER_REFLECT_101`: the edge pixel itself is
/// not repeated, so index `len` maps to `len - 2`.
fn reflect_index(mut i: i32, len: i32) -> usize {
    if len == 1 {
        return 0;
    }
    loop {
        if i < 0 {
            i = -i;
        } else if i >= len {
            i = 2 * (len - 1) - i;
        } else {
            return i as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clip_limit_flattens_a_flat_tile_instead_of_saturating_it() {
        // 8x8 tiles over a 16x16 image are 2x2 pixels, which makes `clipLimit=2.0`
        // work out to a cap of one pixel per bin. Three of a flat tile's four
        // pixels are therefore redistributed across the histogram, and the LUT
        // lands mid-range rather than mapping the one occupied bin to 255.
        let src = Gray::from_vec(16, 16, vec![40; 256]);
        let out = Clahe::detector_default().apply(&src);
        assert!(out.data.iter().all(|&v| v == 128), "got {:?}", &out.data[..8]);
    }

    #[test]
    fn contrast_is_stretched_not_inverted() {
        // A left-dark / right-bright split must stay ordered after equalisation.
        let mut src = Gray::new(32, 32);
        for y in 0..32 {
            for x in 0..32 {
                src.set(x, y, if x < 16 { 90 } else { 130 });
            }
        }
        let out = Clahe::detector_default().apply(&src);
        assert!(out.at(2, 16) < out.at(29, 16));
    }

    #[test]
    fn a_size_that_does_not_divide_by_eight_still_returns_the_original_size() {
        let src = Gray::from_vec(37, 21, (0..37 * 21).map(|i| (i % 251) as u8).collect());
        let out = Clahe::detector_default().apply(&src);
        assert_eq!((out.w, out.h), (37, 21));
    }

    #[test]
    fn clipping_conserves_the_pixel_count() {
        let mut hist = [0i32; HIST_SIZE];
        hist[10] = 900;
        hist[200] = 124;
        let before: i32 = hist.iter().sum();
        clip_histogram(&mut hist, 8);
        assert_eq!(hist.iter().sum::<i32>(), before);
        assert!(hist[10] <= 8 + 900 / HIST_SIZE as i32 + 1);
    }

    #[test]
    fn reflect_101_does_not_repeat_the_edge_pixel() {
        let src = Gray::from_vec(4, 1, vec![1, 2, 3, 4]);
        let out = reflect_101(&src, 2, 0);
        assert_eq!(out.data, vec![1, 2, 3, 4, 3, 2]);
    }
}
