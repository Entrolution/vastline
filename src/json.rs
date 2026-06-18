//! Loose accessors over vast.ai's JSON responses — defensive `.get()` style so a missing key
//! or unexpected type degrades to `None` rather than ever breaking a render. Ported from
//! quotaline's json.rs.

use serde_json::Value;

/// Walk a key path, yielding the value only if every step exists and the leaf is non-null.
pub fn nested<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key)?;
    }
    if cur.is_null() {
        None
    } else {
        Some(cur)
    }
}

/// A JSON number, or a numeric string — `None` for anything else. vast.ai is mostly clean
/// numbers, but `dph_total` has been seen as a string on some endpoints, so we stay loose.
pub fn as_f64_loose(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                t.parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

/// Number at a key path (loose).
pub fn f64_at(v: &Value, path: &[&str]) -> Option<f64> {
    nested(v, path).and_then(as_f64_loose)
}

/// String at a key path.
pub fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    nested(v, path).and_then(|x| x.as_str())
}
