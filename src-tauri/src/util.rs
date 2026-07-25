//! Small shared helpers.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer};

/// Seconds since the Unix epoch as a float, matching Python's `time.time()`.
pub fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Milliseconds since the Unix epoch, matching Python's `int(time.time() * 1000)`
/// used to mint chain/schedule IDs.
pub fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Round to one decimal place, matching Python's `round(x, 1)` for display values.
pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Round to two decimals exactly like Python 3's `round(x, 2)`. Used for the delay
/// steps `macro_to_steps` synthesizes from recording gaps, whose values are both
/// persisted and displayed, so the result must match the source float-for-float.
///
/// Python rounds on the *exact* binary value with ties-to-even. The obvious
/// `(v * 100.0).round() / 100.0` does not: `2.675` is really `2.67499…`, but
/// `2.675 * 100.0` rounds up to exactly `267.5`, which then rounds to `268` →
/// `2.68`, where Python gives `2.67`. Formatting to two places and parsing back
/// avoids the intermediate multiply: Rust's `{:.2}` is correctly rounded on the
/// exact value with ties-to-even, so it reproduces `round(x, 2)` across the tie
/// cases (`0.125→0.12`, `0.375→0.38`, `2.675→2.67`).
pub fn round2(v: f64) -> f64 {
    format!("{v:.2}").parse().expect("a fixed-precision float always reparses")
}

/// The same trick at three decimals — `round(x, 3)`, which is the precision both
/// Test buttons report a match confidence at.
pub fn round3(v: f64) -> f64 {
    format!("{v:.3}").parse().expect("a fixed-precision float always reparses")
}

/// Format a float like Python's `str(float)`: integer-valued floats keep one
/// decimal (`2.0`, not Rust's `2`); everything else uses the shortest round trip.
/// Python interpolates step delays/timeouts and playback speed through `str`, so
/// their messages must render the same way.
pub fn py_float(f: f64) -> String {
    if f.is_finite() && f == f.trunc() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

/// Deserialize a value that may be JSON `null`, coercing null to the type's
/// default. Mirrors Python's `from_dict` guards like
/// `notes = d.get("notes", ""); if not isinstance(notes, str): notes = ""`,
/// where a hand-written `null` falls back to the default instead of erroring.
/// Pair with `#[serde(default, deserialize_with = "null_default")]` so a missing
/// key defaults too.
pub fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Write `bytes` to `path` atomically: create parent dirs, write a sibling
/// `.tmp` file, then rename it over the target. `std::fs::rename` replaces the
/// destination atomically on the same volume (Windows included), so a crash
/// mid-write can never leave a truncated data file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = PathBuf::from(path).into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::{py_float, round2};

    #[test]
    fn py_float_matches_python_str() {
        // str(float): integer-valued floats keep exactly one decimal place.
        assert_eq!(py_float(2.0), "2.0");
        assert_eq!(py_float(1.0), "1.0");
        assert_eq!(py_float(1.5), "1.5");
        assert_eq!(py_float(2.5), "2.5");
        assert_eq!(py_float(0.25), "0.25");
    }

    #[test]
    fn round2_matches_python_round_ties_to_even() {
        assert_eq!(round2(1.567), 1.57);
        assert_eq!(round2(2.0), 2.0);
        assert_eq!(round2(0.5), 0.5);
        assert_eq!(round2(1.005), 1.0); // 1.005 is really 1.00499… in f64 → 1.0
        assert_eq!(round2(2.675), 2.67); // 2.675 is really 2.67499… in f64 → 2.67
        // Exact dyadic ties round to even — the naive `(v*100).round()/100` gets
        // these wrong (and 2.675 too), so they pin the format-parse behaviour.
        assert_eq!(round2(0.125), 0.12);
        assert_eq!(round2(0.375), 0.38);
        assert_eq!(round2(0.625), 0.62);
    }
}
