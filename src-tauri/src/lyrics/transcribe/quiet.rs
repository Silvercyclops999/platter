//! The quiet parts of a separated voice: where nobody sings.
//!
//! Separated from the music, a voice is near silent between lines and over
//! the instrumental parts, apart from a little of the band bleeding through.
//! The speech model makes words up over that bleed and over silence, so the
//! quiet stretches are hushed to true silence before it listens, and any
//! line it still finds there is left out after. Times stay as they were:
//! nothing is cut out of the audio.

use super::audio::RATE;
use super::lines::Segment;

/// Loudness is measured over frames this long.
const FRAME_MS: usize = 20;
/// Quieter than this below the song's loud singing is quiet (dB).
const BELOW_LOUD_DB: f32 = 30.0;
/// Never quiet above this, however loud the singing (dBFS).
const FLOOR_DB: f32 = -55.0;
/// Singing this near a quiet stretch keeps it (breaths, soft word ends).
const KEEP_MS: u32 = 250;
/// Shorter gaps between words aren't hushed.
const SHORTEST_MS: u32 = 500;
/// A line this much inside quiet stretches is made up.
const MOSTLY: f32 = 0.8;

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// The quiet stretches of 16 kHz mono audio, as (start, end) in ms.
pub fn spans(pcm: &[f32]) -> Vec<(u32, u32)> {
    let frame = RATE as usize * FRAME_MS / 1000;
    let levels: Vec<f32> = pcm.chunks(frame).map(|f| db((f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt())).collect();
    if levels.is_empty() {
        return Vec::new();
    }
    // The singing's loud level: near the top, past any odd click.
    let mut sorted = levels.clone();
    sorted.sort_by(f32::total_cmp);
    let loud = sorted[(sorted.len() - 1) * 95 / 100];
    let line = (loud - BELOW_LOUD_DB).max(FLOOR_DB);
    let keep = (KEEP_MS as usize / FRAME_MS) as isize;
    let sung: Vec<bool> = levels.iter().map(|l| *l > line).collect();
    // Quiet unless singing is within reach on either side.
    let mut near = vec![false; sung.len()];
    let mut last: Option<isize> = None;
    for (i, s) in sung.iter().enumerate() {
        if *s {
            last = Some(i as isize);
        }
        near[i] = last.is_some_and(|l| i as isize - l <= keep);
    }
    let mut next: Option<isize> = None;
    for i in (0..sung.len()).rev() {
        if sung[i] {
            next = Some(i as isize);
        }
        near[i] |= next.is_some_and(|n| n - i as isize <= keep);
    }
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..=near.len() {
        let quiet = i < near.len() && !near[i];
        match (quiet, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                let (a, b) = ((s * FRAME_MS) as u32, ((i * FRAME_MS) as u32).min((pcm.len() * 1000 / RATE as usize) as u32));
                if b - a >= SHORTEST_MS {
                    out.push((a, b));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Silence over the quiet stretches.
pub fn hush(pcm: &mut [f32], quiet: &[(u32, u32)]) {
    let len = pcm.len();
    let at = |ms: u32| (ms as usize * RATE as usize / 1000).min(len);
    for (a, b) in quiet {
        let (a, b) = (at(*a), at(*b));
        pcm[a..b].fill(0.0);
    }
}

/// The segments without those found mostly inside quiet stretches.
pub fn drop_made_up(segments: Vec<Segment>, quiet: &[(u32, u32)]) -> Vec<Segment> {
    if quiet.is_empty() {
        return segments;
    }
    segments
        .into_iter()
        .filter(|s| {
            let (Some(first), Some(last)) = (s.words.first(), s.words.last()) else { return true };
            let (a, b) = (first.start_ms, last.end_ms.max(first.start_ms + 1));
            let inside: u32 = quiet.iter().map(|(qa, qb)| b.min(*qb).saturating_sub(a.max(*qa))).sum();
            let made_up = inside as f32 >= MOSTLY * (b - a) as f32;
            if made_up {
                tracing::debug!(start_ms = a, "a line over a quiet part was left out");
            }
            !made_up
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::transcribe::lines::Word;

    /// Seconds of singing (a loud tone) and of bleed (a soft one).
    fn voice(parts: &[(bool, f32)]) -> Vec<f32> {
        let mut out = Vec::new();
        for (sung, secs) in parts {
            let amp = if *sung { 0.3 } else { 0.002 };
            let n = (secs * RATE as f32) as usize;
            out.extend((0..n).map(|i| (i as f32 * 0.05).sin() * amp));
        }
        out
    }

    fn seg(a: u32, b: u32) -> Segment {
        Segment { words: vec![Word { start_ms: a, end_ms: b, text: " la".into() }], sure: None }
    }

    #[test]
    fn the_band_bleeding_through_between_lines_is_quiet() {
        let pcm = voice(&[(true, 2.0), (false, 3.0), (true, 2.0), (false, 0.3), (true, 1.0)]);
        let q = spans(&pcm);
        assert_eq!(q.len(), 1, "a short gap between words isn't: {q:?}");
        let (a, b) = q[0];
        assert!((2_200..=2_300).contains(&a) && (4_700..=4_800).contains(&b), "{q:?}");
    }

    #[test]
    fn hushing_leaves_the_singing_and_the_times() {
        let mut pcm = voice(&[(true, 1.0), (false, 2.0), (true, 1.0)]);
        let len = pcm.len();
        let q = spans(&pcm);
        hush(&mut pcm, &q);
        assert_eq!(pcm.len(), len);
        assert!(pcm[RATE as usize * 2] == 0.0, "the middle is silent");
        assert!(pcm[RATE as usize / 2] != 0.0 && pcm[RATE as usize * 7 / 2] != 0.0, "the singing is kept");
        assert_eq!(spans(&pcm), q, "silence is found where the hush was");
    }

    #[test]
    fn a_line_over_a_quiet_part_is_left_out() {
        let q = vec![(2_000, 5_000)];
        let kept = drop_made_up(vec![seg(500, 1_900), seg(2_500, 4_500), seg(1_500, 3_000)], &q);
        assert_eq!(kept, vec![seg(500, 1_900), seg(1_500, 3_000)], "only the one mostly inside goes");
    }

    #[test]
    fn a_song_with_no_singing_is_all_quiet_and_silence_is_nothing() {
        let pcm = voice(&[(false, 3.0)]);
        assert_eq!(spans(&pcm), vec![(0, 3_000)]);
        assert!(spans(&[]).is_empty());
        assert_eq!(spans(&vec![0.0; RATE as usize]), vec![(0, 1_000)]);
    }
}
