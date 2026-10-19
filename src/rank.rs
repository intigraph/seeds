//! Exact, bounded decimal ordering keys. No floating point or renumbering.
use crate::error::{Result, SdError};

const SCALE: i128 = 1_000_000_000;
const MAX: i128 = 9_000_000_000_000_000_000 * SCALE;

/// Parse up to nine fractional digits, with absolute value at most 9e18.
pub fn parse(s: &str) -> Result<i128> {
    let bad =
        || SdError::usage("rank must be a decimal within +/-9e18 with at most 9 fractional digits");
    let s = s.trim();
    let (negative, rest) = match s.strip_prefix('-') {
        Some(v) => (true, v),
        None => (false, s),
    };
    let (whole, frac) = rest.split_once('.').unwrap_or((rest, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || frac.len() > 9
        || rest.ends_with('.')
    {
        return Err(bad());
    }
    let whole: i128 = whole.parse().map_err(|_| bad())?;
    let fraction: i128 = if frac.is_empty() {
        0
    } else {
        frac.parse().map_err(|_| bad())?
    };
    let n = whole
        .checked_mul(SCALE)
        .and_then(|v| v.checked_add(fraction * 10_i128.pow(9 - frac.len() as u32)))
        .ok_or_else(bad)?;
    if n > MAX {
        return Err(bad());
    }
    Ok(if negative { -n } else { n })
}

/// Canonical decimal text, shared by RDF and JSON.
pub fn format(n: i128) -> String {
    let sign = if n < 0 { "-" } else { "" };
    let n = n.abs();
    let fractional = format!("{:09}", n % SCALE);
    let fractional = fractional.trim_end_matches('0');
    if fractional.is_empty() {
        format!("{sign}{}", n / SCALE)
    } else {
        format!("{sign}{}.{fractional}", n / SCALE)
    }
}

/// An insertion key strictly inside its bounds; exhaustion refuses without edits.
pub fn between(left: Option<i128>, right: Option<i128>) -> Result<String> {
    let n = match (left, right) {
        (Some(l), Some(r)) if r - l > 1 => l + (r - l) / 2,
        (Some(_), Some(_)) => {
            return Err(SdError::refused(
                "rank gap exhausted or tied; assign explicit distinct ranks first",
            ))
        }
        (Some(l), None) => l + SCALE,
        (None, Some(r)) => r - SCALE,
        (None, None) => 0,
    };
    if !(-MAX..=MAX).contains(&n) {
        return Err(SdError::refused("rank boundary exhausted"));
    }
    Ok(format(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_midpoints_and_bounds() {
        for text in ["0", "-3.125", "9000000000000000000", "0.000000001"] {
            assert_eq!(format(parse(text).unwrap()), text);
        }
        assert_eq!(
            between(Some(parse("1").unwrap()), Some(parse("2").unwrap())).unwrap(),
            "1.5"
        );
        assert!(between(Some(0), Some(1)).is_err());
        for bad in [
            "NaN",
            "1e3",
            "",
            ".5",
            "1.",
            "1.0000000001",
            "9000000000000000001",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
