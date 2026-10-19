//! Clocks on a device link. Last writer wins needs clocks that are roughly
//! right, so each hello carries the sender's clock and the link measures
//! how far apart the two are. The merging side of it (the write rule and
//! held far-future entries) is in the store.

/// Two clocks further apart than this are worth a word to the user.
pub const WARN_MS: u64 = 2 * 60_000;

/// How far the other device's clock is ahead of this one's (negative:
/// behind), from its hello.
pub fn offset(their_clock_ms: u64, my_now_ms: u64) -> i64 {
    their_clock_ms as i64 - my_now_ms as i64
}

fn amount(ms: u64) -> String {
    let mins = (ms + 30_000) / 60_000;
    match mins {
        0 | 1 => "a minute".to_string(),
        m if m < 120 => format!("{m} min"),
        m if m < 48 * 60 => format!("{} hours", (m + 30) / 60),
        m => format!("{} days", (m + 12 * 60) / (24 * 60)),
    }
}

/// "Studio PC's clock is about 2 hours fast", when the clocks are far
/// enough apart to matter.
pub fn peer_words(name: &str, offset_ms: i64) -> Option<String> {
    if offset_ms.unsigned_abs() <= WARN_MS {
        return None;
    }
    let way = if offset_ms > 0 { "fast" } else { "slow" };
    Some(format!("{name}'s clock is about {} {way}", amount(offset_ms.unsigned_abs())))
}

/// The same from this device's side: "This device's clock is about 5 min
/// slow. Fix it in Settings > Date and Time".
pub fn own_words(offset_ms: i64) -> Option<String> {
    if offset_ms.unsigned_abs() <= WARN_MS {
        return None;
    }
    // The other device ahead means this one is slow.
    let way = if offset_ms > 0 { "slow" } else { "fast" };
    Some(format!(
        "This device's clock is about {} {way} compared with your other device. Fix it in Settings > Date and Time",
        amount(offset_ms.unsigned_abs())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_a_little_apart_say_nothing() {
        assert_eq!(offset(1_000, 3_000), -2_000);
        assert!(peer_words("Studio PC", 90_000).is_none());
        assert!(own_words(-(WARN_MS as i64)).is_none());
    }

    #[test]
    fn clocks_far_apart_say_by_how_much_and_which_way() {
        assert_eq!(peer_words("Studio PC", 2 * 3_600_000).unwrap(), "Studio PC's clock is about 2 hours fast");
        assert_eq!(peer_words("iPhone", -5 * 60_000).unwrap(), "iPhone's clock is about 5 min slow");
        assert_eq!(peer_words("iPhone", 3 * 86_400_000).unwrap(), "iPhone's clock is about 3 days fast");
        let own = own_words(2 * 3_600_000).unwrap();
        assert!(own.starts_with("This device's clock is about 2 hours slow"), "{own}");
        assert!(!own.contains('\u{2014}'));
    }
}
