//! Lenient serde helpers: Plex sometimes sends numbers as strings (and ids as
//! numbers) depending on endpoint and server version.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s
            .trim()
            .parse::<i64>()
            .ok()
            .or_else(|| s.trim().parse::<f64>().ok().map(|f| f as i64)),
        Value::Bool(b) => Some(*b as i64),
        _ => None,
    }
}

pub fn opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(v.as_ref().and_then(as_i64))
}

pub fn opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    })
}

pub fn opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::String(s)) => Some(s),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    })
}

pub fn string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(opt_string(d)?.unwrap_or_default())
}

pub fn bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let v = Option::<Value>::deserialize(d)?;
    Ok(match v {
        Some(Value::Bool(b)) => b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(Value::String(s)) => matches!(s.trim(), "1" | "true" | "True"),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct T {
        #[serde(default, deserialize_with = "super::opt_i64")]
        a: Option<i64>,
        #[serde(default, deserialize_with = "super::opt_f64")]
        b: Option<f64>,
        #[serde(default, deserialize_with = "super::string")]
        c: String,
        #[serde(default, deserialize_with = "super::bool")]
        d: bool,
    }

    #[test]
    fn accepts_strings_and_numbers() {
        let t: T = serde_json::from_str(r#"{"a":"12","b":"-7.5","c":42,"d":"1"}"#).unwrap();
        assert_eq!(t.a, Some(12));
        assert_eq!(t.b, Some(-7.5));
        assert_eq!(t.c, "42");
        assert!(t.d);
        let t: T = serde_json::from_str(r#"{"a":12,"b":1,"c":"x","d":true}"#).unwrap();
        assert_eq!(t.a, Some(12));
        assert_eq!(t.c, "x");
        let t: T = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(t.a, None);
        assert!(!t.d);
    }
}
