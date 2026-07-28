//! `cv2.matchTemplate(..., TM_CCOEFF_NORMED)`, with and without a mask.
//!
//! The whole correlation is done in exact integer arithmetic: pixels and
//! template weights are bytes, every product fits in `i32`, and only the final
//! normalising divisions are floating point. That is stricter than OpenCV, which
//! correlates through a 32-bit FFT: the scores here are the ones the algebra
//! says they should be, so a threshold the user tuned against cv2 stays valid
//! instead of drifting by a round-off.
//!
//! Correlation is evaluated directly rather than through an FFT. For the
//! region-scoped guards the app is built around it is the faster of the two (no
//! transform of the whole search area to amortise), and it stays exact. The
//! trade shows up in one place: a template of a few hundred pixels a side swept
//! across a *full-screen* search area costs a few hundred milliseconds more than
//! the transform would. Scoping such a guard to a region removes it entirely.

use std::sync::OnceLock;

use rayon::prelude::*;

use super::image::Gray;

/// The pool every correlation runs on.
///
/// Rayon's global pool is one thread per logical core, which is right for a
/// batch job and wrong for this one. A watcher polls forever in the background
/// while the user plays the game it is watching, so a pass that takes the whole
/// machine takes it from them, and the thread it starves first is the UI thread
/// a Start click is waiting on. Half the cores keeps a pass fast enough that the
/// capture is still the limit, and leaves the other half to the app and the game.
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        rayon::ThreadPoolBuilder::new()
            .num_threads((cores / 2).max(1))
            .thread_name(|i| format!("clawmation-vision-{i}"))
            .build()
            .expect("vision thread pool builds from a valid thread count")
    })
}

/// A search area, pre-digested so a whole scale sweep can be run against it
/// without recomputing the window statistics each time.
pub struct Searched {
    w: usize,
    h: usize,
    /// Pixel values and their squares, as correlation inputs.
    px: Vec<i32>,
    px2: Vec<i32>,
    /// Integral images of the two planes above, `(w + 1) * (h + 1)`, so an
    /// unmasked window sum costs four lookups.
    sum: Vec<i64>,
    sum2: Vec<i64>,
}

/// One `matchTemplate` result plane. `f32` deliberately: OpenCV's is `CV_32F`,
/// and thresholds were tuned against the values that precision produces.
pub struct Scores {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Scores {
    /// Every position scoring at or above `threshold`, in row-major order, the
    /// same order `np.where` hands back.
    pub fn above(&self, threshold: f32) -> Vec<(usize, usize, f32)> {
        let mut hits = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let v = self.data[y * self.w + x];
                if v >= threshold {
                    hits.push((x, y, v));
                }
            }
        }
        hits
    }

    /// The highest-scoring position, `cv2.minMaxLoc`'s `maxLoc`. Ties go to the
    /// first in row-major order: OpenCV compares with a strict `>`, and
    /// `Iterator::max_by` would keep the last instead. It matters: a scale sweep
    /// over a flat or repeating region ties constantly, and the coarse peak it
    /// picks decides where the native refine looks.
    pub fn best(&self) -> Option<(usize, usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, &v) in self.data.iter().enumerate() {
            match best {
                Some((_, b)) if v <= b => {}
                _ => best = Some((i, v)),
            }
        }
        best.map(|(i, v)| (i % self.w, i / self.w, v))
    }

    /// The strongest distinct local maxima at or above `threshold`.
    ///
    /// A single `best()` is brittle for a full-screen search: one look-alike can
    /// win the cheap coarse pass, fail native confirmation, and hide the real
    /// target at the same scale. Returning a tiny set of peaks lets the native
    /// pass judge each plausible location while still avoiding the dense cloud
    /// of adjacent scores around one correlation maximum.
    pub fn peaks(&self, threshold: f32, limit: usize) -> Vec<(usize, usize, f32)> {
        if limit == 0 || self.w == 0 || self.h == 0 {
            return Vec::new();
        }
        let mut peaks = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let i = y * self.w + x;
                let v = self.data[i];
                if v < threshold {
                    continue;
                }
                let x0 = x.saturating_sub(1);
                let y0 = y.saturating_sub(1);
                let x1 = (x + 1).min(self.w - 1);
                let y1 = (y + 1).min(self.h - 1);
                let is_peak = (y0..=y1).all(|ny| {
                    (x0..=x1).all(|nx| {
                        let ni = ny * self.w + nx;
                        ni == i || self.data[ni] < v || (self.data[ni] == v && ni > i)
                    })
                });
                if is_peak {
                    peaks.push((x, y, v));
                }
            }
        }
        peaks.sort_by(|a, b| b.2.total_cmp(&a.2));
        peaks.truncate(limit);
        peaks
    }
}

impl Searched {
    pub fn new(img: &Gray) -> Self {
        let (w, h) = (img.w, img.h);
        let px: Vec<i32> = img.data.iter().map(|&v| i32::from(v)).collect();
        let px2: Vec<i32> = px.iter().map(|&v| v * v).collect();
        Self {
            w,
            h,
            sum: integral(&px, w, h),
            sum2: integral(&px2, w, h),
            px,
            px2,
        }
    }

    /// `TM_CCOEFF_NORMED` over the valid region. `None` when the template cannot
    /// fit, which is the same guard the Python detector applies before calling.
    pub fn ccoeff_normed(&self, templ: &Gray, mask: Option<&Gray>) -> Option<Scores> {
        let (tw, th) = (templ.w, templ.h);
        if tw == 0 || th == 0 || tw > self.w || th > self.h {
            return None;
        }
        let (ow, oh) = (self.w - tw + 1, self.h - th + 1);
        match mask {
            Some(m) if m.w == tw && m.h == th => self.masked(templ, m, ow, oh),
            _ => self.plain(templ, ow, oh),
        }
    }

    fn plain(&self, templ: &Gray, ow: usize, oh: usize) -> Option<Scores> {
        let (tw, th) = (templ.w, templ.h);
        let area = (tw * th) as f64;
        let ker: Vec<i32> = templ.data.iter().map(|&v| i32::from(v)).collect();

        let t_sum: f64 = ker.iter().map(|&v| f64::from(v)).sum();
        let t_sum2: f64 = ker.iter().map(|&v| f64::from(v) * f64::from(v)).sum();
        // Σ(T - mean)², i.e. `templSdv² * area`, OpenCV's `templNorm` before it
        // takes the square root.
        let t_var = t_sum2 - t_sum * t_sum / area;
        if t_var < f64::EPSILON {
            // A featureless template correlates with everything equally; OpenCV
            // says so by returning all ones rather than dividing by zero.
            return Some(Scores {
                w: ow,
                h: oh,
                data: vec![1.0; ow * oh],
            });
        }
        let t_norm = t_var.sqrt();
        let t_mean = t_sum / area;

        let raw = correlate(&self.px, self.w, self.h, &ker, tw, th);
        let mut data = vec![0f32; ow * oh];
        for y in 0..oh {
            for x in 0..ow {
                let wnd = box_sum(&self.sum, self.w, x, y, tw, th) as f64;
                let wnd2 = box_sum(&self.sum2, self.w, x, y, tw, th) as f64;
                let num = raw[y * ow + x] as f64 - t_mean * wnd;
                let diff2 = wnd2 - wnd * wnd / area;
                data[y * ow + x] = normalise(num, diff2, wnd2, t_norm);
            }
        }
        Some(Scores { w: ow, h: oh, data })
    }

    /// The masked form. With a binary mask this reduces to Pearson correlation
    /// over the selected pixels, which is exactly what OpenCV documents:
    /// `T' = M(T - Σ(T·M)/ΣM)` against `I' = M(I - Σ(I·M)/ΣM)`.
    fn masked(&self, templ: &Gray, mask: &Gray, ow: usize, oh: usize) -> Option<Scores> {
        let (tw, th) = (templ.w, templ.h);
        // OpenCV thresholds an 8-bit mask to 0/1 before using it, so any nonzero
        // weight counts the same.
        let m: Vec<i32> = mask.data.iter().map(|&v| i32::from(v != 0)).collect();
        let m_count: f64 = m.iter().map(|&v| f64::from(v)).sum();
        if m_count < 1.0 {
            return None;
        }
        let mt: Vec<i32> = templ
            .data
            .iter()
            .zip(&m)
            .map(|(&t, &k)| i32::from(t) * k)
            .collect();

        let tm_sum: f64 = mt.iter().map(|&v| f64::from(v)).sum();
        let tm_sum2: f64 = mt.iter().map(|&v| f64::from(v) * f64::from(v)).sum();
        let t_var = tm_sum2 - tm_sum * tm_sum / m_count;
        if t_var < f64::EPSILON {
            return Some(Scores {
                w: ow,
                h: oh,
                data: vec![1.0; ow * oh],
            });
        }
        let t_norm = t_var.sqrt();
        let t_mean = tm_sum / m_count;

        let raw = correlate(&self.px, self.w, self.h, &mt, tw, th);
        // The mask replaces the integral images: window sums have to be taken
        // over the selected pixels only, which is a correlation of its own.
        let wnd_sum = correlate(&self.px, self.w, self.h, &m, tw, th);
        let wnd_sum2 = correlate(&self.px2, self.w, self.h, &m, tw, th);

        let mut data = vec![0f32; ow * oh];
        for i in 0..ow * oh {
            let wnd = wnd_sum[i] as f64;
            let wnd2 = wnd_sum2[i] as f64;
            let num = raw[i] as f64 - t_mean * wnd;
            let diff2 = wnd2 - wnd * wnd / m_count;
            data[i] = normalise(num, diff2, wnd2, t_norm);
        }
        Some(Scores { w: ow, h: oh, data })
    }
}

/// OpenCV's normalising epilogue, quirks included. The `1.125` band exists
/// because a numerator that overshoots its own denominator can only be
/// round-off on a perfect match; past that band the window is degenerate and the
/// score is thrown away rather than reported as a huge number.
fn normalise(num: f64, diff2: f64, wnd2: f64, t_norm: f64) -> f32 {
    let diff2 = diff2.max(0.0);
    let t = if diff2 <= (0.5f64).min(10.0 * f64::from(f32::EPSILON) * wnd2) {
        0.0
    } else {
        diff2.sqrt() * t_norm
    };
    let v = if num.abs() < t {
        num / t
    } else if num.abs() < t * 1.125 {
        if num > 0.0 {
            1.0
        } else {
            -1.0
        }
    } else {
        0.0
    };
    v as f32
}

/// Valid-region cross-correlation: `out[y][x] = Σ ker[j][i] · img[y + j][x + i]`.
///
/// Row products are accumulated in `i32`, which is exact as long as no single
/// product exceeds `255²`, true for every plane this module feeds it: pixels
/// against template bytes, and squared pixels against a 0/1 mask.
fn correlate(img: &[i32], iw: usize, ih: usize, ker: &[i32], kw: usize, kh: usize) -> Vec<i64> {
    debug_assert!(kw <= iw && kh <= ih);
    let (ow, oh) = (iw - kw + 1, ih - kh + 1);
    let mut out = vec![0i64; ow * oh];
    pool().install(|| {
        out.par_chunks_mut(ow).enumerate().for_each(|(y, row)| {
            for (x, cell) in row.iter_mut().enumerate() {
                let mut acc = 0i64;
                for j in 0..kh {
                    let irow = &img[(y + j) * iw + x..][..kw];
                    let krow = &ker[j * kw..][..kw];
                    let mut s = 0i32;
                    for i in 0..kw {
                        s += krow[i] * irow[i];
                    }
                    acc += i64::from(s);
                }
                *cell = acc;
            }
        });
    });
    out
}

/// Summed-area table, one row and column of zeros wider than the source.
fn integral(px: &[i32], w: usize, h: usize) -> Vec<i64> {
    let mut out = vec![0i64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut run = 0i64;
        for x in 0..w {
            run += i64::from(px[y * w + x]);
            out[(y + 1) * (w + 1) + x + 1] = out[y * (w + 1) + x + 1] + run;
        }
    }
    out
}

fn box_sum(sat: &[i64], w: usize, x: usize, y: usize, bw: usize, bh: usize) -> i64 {
    let stride = w + 1;
    sat[y * stride + x] + sat[(y + bh) * stride + x + bw]
        - sat[y * stride + x + bw]
        - sat[(y + bh) * stride + x]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gray(w: usize, h: usize, v: &[u8]) -> Gray {
        Gray::from_vec(w, h, v.to_vec())
    }

    #[test]
    fn a_template_lifted_out_of_the_image_scores_one_where_it_came_from() {
        let img = gray(
            6,
            4,
            &[
                10, 20, 30, 40, 50, 60, //
                11, 90, 91, 44, 55, 66, //
                12, 92, 93, 45, 56, 67, //
                13, 24, 34, 46, 57, 68,
            ],
        );
        let templ = gray(2, 2, &[90, 91, 92, 93]);
        let s = Searched::new(&img).ccoeff_normed(&templ, None).unwrap();
        assert_eq!((s.w, s.h), (5, 3));
        let (bx, by, bv) = s.best().unwrap();
        assert_eq!((bx, by), (1, 1));
        assert!((bv - 1.0).abs() < 1e-5, "peak scored {bv}");
    }

    #[test]
    fn brightness_and_contrast_shifts_do_not_move_the_peak() {
        // CCOEFF_NORMED is invariant to `a·I + b`; that invariance is the whole
        // reason the detector uses it over a plain difference.
        let base: Vec<u8> = (0..8 * 8).map(|i| ((i * 7) % 200) as u8).collect();
        let img = gray(8, 8, &base);
        let bright: Vec<u8> = base.iter().map(|&v| (v / 2 + 40) as u8).collect();
        let templ = gray(8, 8, &bright);
        let s = Searched::new(&img).ccoeff_normed(&templ, None).unwrap();
        assert!((s.data[0] - 1.0).abs() < 1e-4, "scored {}", s.data[0]);
    }

    #[test]
    fn an_inverted_patch_scores_minus_one() {
        let base: Vec<u8> = (0..6 * 6).map(|i| ((i * 5) % 250) as u8).collect();
        let img = gray(6, 6, &base);
        let inverted: Vec<u8> = base.iter().map(|&v| 255 - v).collect();
        let s = Searched::new(&img)
            .ccoeff_normed(&gray(6, 6, &inverted), None)
            .unwrap();
        assert!((s.data[0] + 1.0).abs() < 1e-4, "scored {}", s.data[0]);
    }

    #[test]
    fn a_flat_template_scores_one_everywhere() {
        let img = gray(5, 5, &(0..25).map(|i| (i * 9) as u8).collect::<Vec<_>>());
        let s = Searched::new(&img)
            .ccoeff_normed(&gray(2, 2, &[7, 7, 7, 7]), None)
            .unwrap();
        assert!(s.data.iter().all(|&v| v == 1.0));
    }

    #[test]
    fn a_mask_ignores_the_pixels_it_clears() {
        // The template's right column is garbage; masking it off must restore a
        // perfect match on the left column, which does come from the image.
        let img = gray(4, 2, &[10, 200, 30, 40, 60, 12, 90, 80]);
        let templ = gray(2, 2, &[10, 7, 60, 250]);
        let mask = gray(2, 2, &[255, 0, 255, 0]);
        let searched = Searched::new(&img);
        let masked = searched.ccoeff_normed(&templ, Some(&mask)).unwrap();
        assert!(
            (masked.data[0] - 1.0).abs() < 1e-4,
            "masked scored {}",
            masked.data[0]
        );
        let plain = searched.ccoeff_normed(&templ, None).unwrap();
        assert!(
            plain.data[0] < 0.95,
            "unmasked should not match: {}",
            plain.data[0]
        );
    }

    #[test]
    fn a_template_larger_than_the_search_area_is_refused() {
        let img = gray(3, 3, &[0; 9]);
        assert!(Searched::new(&img)
            .ccoeff_normed(&gray(4, 2, &[0; 8]), None)
            .is_none());
    }

    #[test]
    fn above_returns_hits_in_row_major_order() {
        let s = Scores {
            w: 3,
            h: 2,
            data: vec![0.1, 0.9, 0.2, 0.8, 0.05, 0.95],
        };
        let hits: Vec<(usize, usize)> = s.above(0.8).into_iter().map(|(x, y, _)| (x, y)).collect();
        assert_eq!(hits, vec![(1, 0), (0, 1), (2, 1)]);
    }

    #[test]
    fn best_breaks_ties_towards_the_first_position() {
        let s = Scores {
            w: 3,
            h: 2,
            data: vec![0.2, 0.9, 0.5, 0.9, 0.1, 0.9],
        };
        assert_eq!(s.best().map(|(x, y, _)| (x, y)), Some((1, 0)));
    }

    #[test]
    fn peaks_keep_distinct_candidates_without_adjacent_duplicates() {
        let s = Scores {
            w: 5,
            h: 3,
            data: vec![
                0.1, 0.80, 0.79, 0.1, 0.1, //
                0.1, 0.78, 0.77, 0.1, 0.91, //
                0.1, 0.1, 0.1, 0.1, 0.89,
            ],
        };
        assert_eq!(s.peaks(0.7, 4), vec![(4, 1, 0.91), (1, 0, 0.80)]);
    }

    #[test]
    fn the_integral_image_agrees_with_a_direct_sum() {
        let w = 7;
        let h = 5;
        let px: Vec<i32> = (0..(w * h) as i32).map(|i| i * 3 % 91).collect();
        let sat = integral(&px, w, h);
        for (x, y, bw, bh) in [(0usize, 0usize, 7usize, 5usize), (2, 1, 3, 3), (6, 4, 1, 1)] {
            let mut direct: i64 = 0;
            for j in y..y + bh {
                for i in x..x + bw {
                    direct += i64::from(px[j * w + i]);
                }
            }
            assert_eq!(
                box_sum(&sat, w, x, y, bw, bh),
                direct,
                "box {x},{y} {bw}x{bh}"
            );
        }
    }
}
