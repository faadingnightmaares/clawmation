//! Text detection through the Windows OCR engine (`Windows.Media.Ocr`).
//!
//! Replaces the sidecar's EasyOCR path, which never actually shipped: `easyocr`
//! pulls in torch, so it was excluded from the frozen sidecar (MIGRATION-NOTES,
//! "Intentionally not ported") and every `method = "ocr"` guard died on
//! `ModuleNotFoundError` at first use. The platform engine is already on every
//! Windows 10/11 install, needs no model download, adds nothing to the bundle,
//! and answers in milliseconds rather than seconds.
//!
//! The observable contract is the one `detection.py::ocr_find` published, because
//! the guard engine and the UI are already written against it: coordinates are
//! the match *centre*, absolute (region origin already added), and matching is a
//! case-insensitive substring test.

use std::sync::{Mutex, OnceLock};

use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine as WinOcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;

use super::capture::Frame;
use super::vision::Detection;

/// The recognizer wants a reasonable number of pixels per glyph. Screen text in
/// a tight guard region is often 10-14px tall, which it reads poorly or not at
/// all, so anything smaller than this on its short side is scaled up first.
/// Purely an input transform; reported coordinates are always divided back.
const MIN_SHORT_SIDE: u32 = 160;

/// Ceiling on that upscale, so a 1x4 sliver can't ask for a giant allocation.
const MAX_SCALE: u32 = 6;

/// A word the engine read, in the coordinate space of the frame handed to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Why OCR could not run. Distinct from a *successful* read that found nothing:
/// the guard loop logs this once and carries on, rather than treating a missing
/// language pack as "text absent".
#[derive(Debug)]
pub enum OcrError {
    /// No recognizer for any of the user's languages: no OCR language pack.
    Unavailable,
    /// The WinRT call itself failed.
    Windows(windows::core::Error),
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(
                f,
                "no Windows OCR language pack is installed (Settings › Time & language › Language)"
            ),
            Self::Windows(e) => write!(f, "Windows OCR error: {e}"),
        }
    }
}

impl std::error::Error for OcrError {}

impl From<windows::core::Error> for OcrError {
    fn from(e: windows::core::Error) -> Self {
        Self::Windows(e)
    }
}

/// The process-wide engine. Creating one costs a few milliseconds and it is
/// immutable once built, so it is made once and shared; the `Mutex` exists only
/// because `IOcrEngine` is not `Sync`, and recognition is serialised anyway by
/// the single guard poll loop.
fn engine() -> Result<&'static Mutex<WinOcrEngine>, OcrError> {
    static ENGINE: OnceLock<Option<Mutex<WinOcrEngine>>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            WinOcrEngine::TryCreateFromUserProfileLanguages()
                .ok()
                .map(Mutex::new)
        })
        .as_ref()
        .ok_or(OcrError::Unavailable)
}

/// True when this machine can OCR at all, used to report the capability
/// honestly in status rather than silently returning "no match".
pub fn available() -> bool {
    engine().is_ok()
}

/// Integer upscale factor that lifts the short side to `MIN_SHORT_SIDE`.
/// 1 when the frame is already big enough (the common full-screen case), so the
/// hot path allocates nothing extra.
fn scale_factor(width: u32, height: u32) -> u32 {
    let short = width.min(height);
    if short == 0 || short >= MIN_SHORT_SIDE {
        return 1;
    }
    MIN_SHORT_SIDE.div_ceil(short).min(MAX_SCALE)
}

/// Nearest-neighbour upscale of a tight BGR buffer by an integer factor.
///
/// Nearest, not bilinear: screen text is already crisp, aliased glyph edges, and
/// interpolating them smears the stems the recognizer keys on. Replicating
/// pixels keeps the edges hard and only gives the engine more of them.
fn upscale_bgr(bgr: &[u8], width: u32, height: u32, factor: u32) -> Vec<u8> {
    let (w, h, f) = (width as usize, height as usize, factor as usize);
    let mut out = vec![0u8; w * f * h * f * 3];
    let dst_stride = w * f * 3;
    for y in 0..h {
        // Build one destination row, then copy it `f` times.
        let row_start = y * f * dst_stride;
        for x in 0..w {
            let src = (y * w + x) * 3;
            let px = &bgr[src..src + 3];
            for rep in 0..f {
                let dst = row_start + (x * f + rep) * 3;
                out[dst..dst + 3].copy_from_slice(px);
            }
        }
        for dup in 1..f {
            let (head, tail) = out.split_at_mut(row_start + dup * dst_stride);
            tail[..dst_stride].copy_from_slice(&head[row_start..row_start + dst_stride]);
        }
    }
    out
}

/// Tight BGR → tight BGRA with an opaque alpha channel.
///
/// The alpha matters: `SoftwareBitmap` treats a zero alpha as fully transparent
/// and the recognizer then reads a blank image, so every pixel is forced to 255.
fn bgr_to_bgra(bgr: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bgr.len() / 3 * 4);
    for px in bgr.chunks_exact(3) {
        out.extend_from_slice(px);
        out.push(0xFF);
    }
    out
}

/// Read every word in `frame`, in reading order. Coordinates come back in the
/// frame's own pixel space with any internal upscale already divided out.
pub fn read_words(frame: &Frame) -> Result<Vec<Word>, OcrError> {
    Ok(read_lines(frame)?.into_iter().flatten().collect())
}

/// Recognize `frame`, keeping the engine's own line grouping so a multi-word
/// phrase can be matched across the spaces between words.
///
/// Public because recognizing is by far the most expensive thing the watcher
/// does: callers with several needles over the same area read the lines once and
/// run every needle past them with [`match_lines`], instead of paying for one
/// full recognition per needle.
pub fn read_lines(frame: &Frame) -> Result<Vec<Vec<Word>>, OcrError> {
    if frame.width == 0 || frame.height == 0 {
        return Ok(Vec::new());
    }
    let factor = scale_factor(frame.width, frame.height);
    // At factor 1 (the full-screen case), the frame's own buffer is handed
    // straight to the converter. Cloning it first would copy several megabytes
    // per pass for nothing.
    let upscaled;
    let (bgr, w, h) = if factor > 1 {
        upscaled = upscale_bgr(&frame.bgr, frame.width, frame.height, factor);
        (&upscaled[..], frame.width * factor, frame.height * factor)
    } else {
        (&frame.bgr[..], frame.width, frame.height)
    };

    let bgra = bgr_to_bgra(bgr);
    let buffer = CryptographicBuffer::CreateFromByteArray(&bgra)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        w as i32,
        h as i32,
    )?;

    let result = {
        let engine = engine()?.lock().unwrap();
        engine.RecognizeAsync(&bitmap)?.get()?
    };

    let inv = f64::from(factor);
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let mut words = Vec::new();
        for word in line.Words()? {
            let r = word.BoundingRect()?;
            words.push(Word {
                text: word.Text()?.to_string_lossy(),
                x: f64::from(r.X) / inv,
                y: f64::from(r.Y) / inv,
                w: f64::from(r.Width) / inv,
                h: f64::from(r.Height) / inv,
            });
        }
        if !words.is_empty() {
            lines.push(words);
        }
    }
    Ok(lines)
}

/// Fold a string to the form both sides of a match are compared in: lowercase,
/// runs of whitespace collapsed to one space, ends trimmed. Without this a guard
/// for "play again" misses a line the engine read as "Play  Again".
fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// How many character errors a needle of `len` characters may absorb.
///
/// The recognizer is accurate on crisp text and unreliable on the outlined,
/// textured lettering games use: it turns an l into an I, drops the tail off a
/// t, reads a 0 as an O. A guard for "Reconnect" that is thrown away over one
/// such character is the difference between a trigger that works and one the
/// user gives up on. Short needles get no slack at all, because at three or four
/// characters a single edit is most of the word.
fn edit_budget(len: usize) -> usize {
    if len < 6 {
        0
    } else {
        (len / 6).min(3)
    }
}

/// Levenshtein distance between `needle` and the best-matching substring of
/// `hay` ending at each position (Sellers' variant, where the start of the
/// match is free). Returns the end offset of the closest match within `budget`,
/// preferring fewer errors and then the earlier end.
fn approx_end(hay: &[char], needle: &[char], budget: usize) -> Option<(usize, usize)> {
    let m = needle.len();
    let mut dp: Vec<usize> = (0..=m).collect();
    let mut best: Option<(usize, usize)> = None;
    for i in 1..=hay.len() {
        // dp[j] is rewritten in place, so the pre-update value of dp[j-1] (the
        // diagonal) has to be carried forward by hand.
        let mut diag = dp[0];
        dp[0] = 0; // a match may start anywhere
        for j in 1..=m {
            let up = dp[j];
            let cost = usize::from(hay[i - 1] != needle[j - 1]);
            dp[j] = (diag + cost).min(up + 1).min(dp[j - 1] + 1);
            diag = up;
        }
        if dp[m] <= budget && best.is_none_or(|(_, d)| dp[m] < d) {
            best = Some((i, dp[m]));
        }
    }
    best
}

/// Locate `needle` in `hay`, exactly if it is there and within
/// [`edit_budget`] character errors otherwise. Returns the `[start, end)` range
/// of the match in `hay` in character offsets, plus how many characters were
/// wrong (0 for an exact hit).
///
/// The end is found by scanning forwards; the start by running the same scan
/// over both strings reversed, so the reported span is the actual matched text
/// rather than a guess at where it must have begun.
fn find_fuzzy(hay: &str, needle: &str) -> Option<(usize, usize, usize)> {
    if let Some(byte) = hay.find(needle) {
        let start = hay[..byte].chars().count();
        return Some((start, start + needle.chars().count(), 0));
    }
    let hay: Vec<char> = hay.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    let budget = edit_budget(needle.len());
    if budget == 0 || needle.len() > hay.len() + budget {
        return None;
    }
    let (end, dist) = approx_end(&hay, &needle, budget)?;

    let head: Vec<char> = hay[..end].iter().rev().copied().collect();
    let tail: Vec<char> = needle.iter().rev().copied().collect();
    let (back, _) = approx_end(&head, &tail, budget)?;
    Some((end - back, end, dist))
}

/// Which words of a line are covered by the match at `[start, end)` in the
/// line's normalized text, given each word's start offset in that text.
fn covered<'a>(
    words: &'a [Word],
    offsets: &[(usize, usize)],
    start: usize,
    end: usize,
) -> Vec<&'a Word> {
    words
        .iter()
        .zip(offsets)
        .filter(|(_, (ws, we))| *ws < end && *we > start)
        .map(|(w, _)| w)
        .collect()
}

/// The union bounding box of a set of words.
fn union(words: &[&Word]) -> (f64, f64, f64, f64) {
    let x0 = words.iter().map(|w| w.x).fold(f64::MAX, f64::min);
    let y0 = words.iter().map(|w| w.y).fold(f64::MAX, f64::min);
    let x1 = words.iter().map(|w| w.x + w.w).fold(f64::MIN, f64::max);
    let y1 = words.iter().map(|w| w.y + w.h).fold(f64::MIN, f64::max);
    (x0, y0, x1 - x0, y1 - y0)
}

/// Find `needle` in `frame` and return one detection per matching line.
///
/// `origin` is the region's top-left in screen space, added to every result so
/// callers get absolute coordinates, the same `roi_offset` contract the sidecar's
/// detections carried. Matching runs over the whitespace-normalized line, so a
/// phrase spanning several words still matches, and tolerates a few misread
/// characters (see [`find_fuzzy`]). An empty `needle` returns every line,
/// mirroring `ocr_find`'s `if target and`.
pub fn find_text(
    frame: &Frame,
    needle: &str,
    origin: (i64, i64),
) -> Result<Vec<Detection>, OcrError> {
    Ok(match_lines(&read_lines(frame)?, needle, origin))
}

/// The matching half of [`find_text`], over lines already read by
/// [`read_lines`]. Pure and cheap (string work over a handful of words), so a
/// caller with several needles pays for recognition once.
pub fn match_lines(lines: &[Vec<Word>], needle: &str, origin: (i64, i64)) -> Vec<Detection> {
    let target = normalize(needle);
    let (ox, oy) = origin;
    let mut out = Vec::new();

    for words in lines {
        // Rebuild the line exactly as `normalize` would see it, remembering where
        // each word landed so a hit maps back to the words that produced it.
        // Offsets are counted in characters, the unit `find_fuzzy` reports in.
        let mut text = String::new();
        let mut chars = 0usize;
        let mut offsets = Vec::with_capacity(words.len());
        for word in words {
            let piece = normalize(&word.text);
            if piece.is_empty() {
                offsets.push((chars, chars));
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
                chars += 1;
            }
            let start = chars;
            text.push_str(&piece);
            chars += piece.chars().count();
            offsets.push((start, chars));
        }

        let (hits, wrong): (Vec<&Word>, usize) = if target.is_empty() {
            (words.iter().collect(), 0)
        } else {
            match find_fuzzy(&text, &target) {
                Some((at, end, dist)) => (covered(words, &offsets, at, end), dist),
                None => continue,
            }
        };
        if hits.is_empty() {
            continue;
        }

        let (x, y, w, h) = union(&hits);
        out.push(Detection {
            label: hits
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            x: ox + (x + w / 2.0).round() as i64,
            y: oy + (y + h / 2.0).round() as i64,
            w: (w.round() as i64).max(1),
            h: (h.round() as i64).max(1),
            // The platform engine exposes no per-word score, so this reports how
            // much of the needle was read verbatim: 1.0 for an exact hit, less
            // when characters had to be forgiven.
            confidence: 1.0 - wrong as f64 / target.chars().count().max(1) as f64,
            roi_offset: [ox, oy],
            scale_x: 1.0,
            scale_y: 1.0,
            observation: None,
        });
    }
    out
}

/// Every word in `frame`, joined: the port of `ocr_region`.
pub fn read_text(frame: &Frame) -> Result<String, OcrError> {
    Ok(read_words(frame)?
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32) -> Frame {
        Frame {
            bgr: vec![0u8; (width * height * 3) as usize],
            width,
            height,
        }
    }

    #[test]
    fn scale_factor_leaves_large_frames_alone() {
        assert_eq!(scale_factor(1920, 1080), 1);
        assert_eq!(scale_factor(200, 160), 1);
    }

    #[test]
    fn scale_factor_lifts_small_frames_to_the_minimum() {
        // 40px short side needs 4x to clear 160.
        assert_eq!(scale_factor(400, 40), 4);
        // Ceiling, not floor: 50px would be 3.2x, so 4x.
        assert_eq!(scale_factor(400, 50), 4);
    }

    #[test]
    fn scale_factor_is_capped_and_degenerate_safe() {
        assert_eq!(scale_factor(400, 1), MAX_SCALE);
        assert_eq!(scale_factor(0, 0), 1);
    }

    #[test]
    fn upscale_replicates_every_pixel() {
        // 2x1 image: red then blue (BGR order), doubled.
        let src = vec![0, 0, 255, 255, 0, 0];
        let out = upscale_bgr(&src, 2, 1, 2);
        assert_eq!(out.len(), 4 * 2 * 3);
        assert_eq!(&out[0..3], &[0, 0, 255]);
        assert_eq!(&out[3..6], &[0, 0, 255]);
        assert_eq!(&out[6..9], &[255, 0, 0]);
        assert_eq!(&out[9..12], &[255, 0, 0]);
        // Second row duplicates the first.
        assert_eq!(&out[12..24], &out[0..12]);
    }

    #[test]
    fn bgra_conversion_forces_opaque_alpha() {
        let out = bgr_to_bgra(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(out, vec![1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
    }

    #[test]
    fn normalize_folds_case_and_whitespace() {
        assert_eq!(normalize("  Play   AGAIN \n"), "play again");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn covered_picks_only_overlapping_words() {
        let words = vec![
            Word {
                text: "play".into(),
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 4.0,
            },
            Word {
                text: "again".into(),
                x: 12.0,
                y: 0.0,
                w: 12.0,
                h: 4.0,
            },
            Word {
                text: "now".into(),
                x: 26.0,
                y: 0.0,
                w: 8.0,
                h: 4.0,
            },
        ];
        // "play again now": offsets of each word in the joined text.
        let offsets = vec![(0, 4), (5, 10), (11, 14)];
        let hit = covered(&words, &offsets, 0, 10);
        assert_eq!(hit.len(), 2);
        assert_eq!(hit[0].text, "play");
        assert_eq!(hit[1].text, "again");
    }

    #[test]
    fn fuzzy_match_prefers_the_exact_hit() {
        assert_eq!(
            find_fuzzy("press play again now", "play again"),
            Some((6, 16, 0))
        );
        assert_eq!(find_fuzzy("reconnect", "reconnect"), Some((0, 9, 0)));
    }

    #[test]
    fn fuzzy_match_forgives_a_misread_character() {
        // Substitution, deletion and insertion (the three ways a glyph goes
        // wrong) each land on the same word with one error charged.
        for line in ["click recannect", "click reconect", "click recoonnect"] {
            let (start, end, dist) = find_fuzzy(line, "reconnect").expect(line);
            assert_eq!(dist, 1, "{line}");
            assert_eq!(&line[start..end], &line[6..], "{line}");
        }
    }

    #[test]
    fn fuzzy_match_refuses_a_different_word() {
        // Nine characters buys one error, and these are all further away than that.
        assert_eq!(find_fuzzy("connected to server", "reconnect"), None);
        assert_eq!(find_fuzzy("disband party", "reconnect"), None);
    }

    #[test]
    fn short_needles_must_be_exact() {
        // One edit is most of a four-character word, so "quit" may not match "quiz".
        assert_eq!(edit_budget(4), 0);
        assert_eq!(find_fuzzy("take the quiz", "quit"), None);
        // The budget grows with length but never past three.
        assert_eq!(edit_budget(6), 1);
        assert_eq!(edit_budget(12), 2);
        assert_eq!(edit_budget(120), 3);
    }

    #[test]
    fn union_spans_all_words() {
        let a = Word {
            text: "a".into(),
            x: 4.0,
            y: 2.0,
            w: 6.0,
            h: 4.0,
        };
        let b = Word {
            text: "b".into(),
            x: 12.0,
            y: 3.0,
            w: 5.0,
            h: 6.0,
        };
        assert_eq!(union(&[&a, &b]), (4.0, 2.0, 13.0, 7.0));
    }

    #[test]
    fn empty_frame_reads_nothing_without_touching_the_engine() {
        // Guards the zero-size early return: no OCR engine is required, so this
        // passes on a machine with no language pack.
        assert_eq!(read_words(&frame(0, 0)).unwrap(), Vec::new());
        assert_eq!(find_text(&frame(10, 0), "x", (0, 0)).unwrap(), Vec::new());
    }

    /// End-to-end against the live desktop: grabs the real screen and reads it.
    /// Ignored by default because it needs a display, an OCR language pack, and
    /// legible text actually on screen; run it with `--ignored` to check the
    /// recognizer on this machine.
    #[test]
    #[ignore = "requires a display with readable text and an OCR language pack"]
    fn reads_text_off_the_live_screen() {
        use crate::hardware::capture::ScreenCapture;

        assert!(available(), "no OCR language pack installed");
        let mut cap = ScreenCapture::new("dxcam", None);
        let frame = cap.grab().expect("screen grab failed");
        let words = read_words(&frame).expect("OCR failed");
        println!(
            "read {} words from {}x{}",
            words.len(),
            frame.width,
            frame.height
        );
        println!(
            "sample: {:?}",
            words.iter().take(25).map(|w| &w.text).collect::<Vec<_>>()
        );
        assert!(!words.is_empty(), "OCR read nothing off a full desktop");
    }
}
