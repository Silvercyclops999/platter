//! Line timing against ground truth. `fixtures/timing-medium.json` holds the
//! token times whisper.cpp gave for eight spoken lines (our own words, read
//! by the computer's built-in voices) placed at known times, once alone and
//! once over a synthetic music bed. Only the times and words are kept, no
//! audio. The real-model run behind it is `real_model_timing` in tests.rs.

use super::engine::{RawToken, segments_from_tokens};
use super::lines::{Line, Segment, Word, retime_v1, to_lines, to_lrc};
use crate::plex::details::parse_lyrics_text;

/// Line starts must land this close to the truth.
const TOLERANCE_MS: i64 = 300;

struct Run {
    name: String,
    truth: Vec<(String, u32)>,
    segments: Vec<Vec<RawToken>>,
}

fn runs() -> Vec<Run> {
    let v: serde_json::Value = serde_json::from_str(include_str!("fixtures/timing-medium.json")).unwrap();
    v["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| Run {
            name: r["name"].as_str().unwrap().to_owned(),
            truth: r["truth"].as_array().unwrap().iter().map(|t| (t[0].as_str().unwrap().to_owned(), t[1].as_u64().unwrap() as u32)).collect(),
            segments: r["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| {
                    s.as_array()
                        .unwrap()
                        .iter()
                        .map(|t| {
                            let dtw = t[3].as_i64().unwrap();
                            RawToken {
                                bytes: t[0].as_str().unwrap().as_bytes().to_vec(),
                                start_ms: t[1].as_u64().unwrap() as u32,
                                end_ms: t[2].as_u64().unwrap() as u32,
                                dtw_ms: (dtw >= 0).then_some(dtw as u32),
                            }
                        })
                        .collect()
                })
                .collect(),
        })
        .collect()
}

fn fold(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Each true line's start error (found minus true), matching lines by their
/// first two words.
pub(super) fn errors(truth: &[(String, u32)], starts: &[(String, u32)]) -> Vec<i64> {
    truth
        .iter()
        .map(|(text, at)| {
            let want: Vec<String> = text.split(' ').take(2).map(fold).collect();
            let found = starts
                .iter()
                .find(|(t, _)| t.split(' ').take(2).map(fold).collect::<Vec<_>>() == want)
                .unwrap_or_else(|| panic!("no line for {text:?} in {starts:?}"));
            found.1 as i64 - *at as i64
        })
        .collect()
}

pub(super) fn line_starts(lines: &[Line]) -> Vec<(String, u32)> {
    lines.iter().map(|l| (l.text(), l.start_ms)).collect()
}

/// The first rule: a word at its first token's attention time.
fn first_rule(segments: &[Vec<RawToken>]) -> Vec<Segment> {
    segments
        .iter()
        .map(|seg| {
            let mut words: Vec<Word> = Vec::new();
            for t in seg {
                let text = String::from_utf8_lossy(&t.bytes).into_owned();
                if words.is_empty() || text.starts_with(' ') {
                    let at = t.dtw_ms.unwrap_or(t.start_ms);
                    words.push(Word { start_ms: at, end_ms: t.end_ms.max(at), text });
                } else if let Some(w) = words.last_mut() {
                    w.text.push_str(&text);
                    w.end_ms = w.end_ms.max(t.end_ms);
                }
            }
            Segment { words }
        })
        .collect()
}

fn mean(v: &[i64]) -> f64 {
    v.iter().sum::<i64>() as f64 / v.len() as f64
}

#[test]
fn line_starts_land_within_the_tolerance() {
    for run in runs() {
        let lines = to_lines(&segments_from_tokens(&run.segments));
        let e = errors(&run.truth, &line_starts(&lines));
        println!("{}: errors {e:?} ms, mean {:.0}", run.name, mean(&e));
        assert!(e.iter().all(|x| x.abs() <= TOLERANCE_MS), "{}: {e:?}", run.name);
        assert!(mean(&e).abs() <= 120.0, "{}: no lean either way: {e:?}", run.name);
    }
}

#[test]
fn the_first_rule_was_late() {
    // The cause: attention times mark where a token ends.
    for run in runs() {
        let e = errors(&run.truth, &line_starts(&to_lines(&first_rule(&run.segments))));
        println!("{} (first rule): errors {e:?} ms, mean {:.0}", run.name, mean(&e));
        assert!(e.iter().all(|x| *x > 0), "every line came late: {e:?}");
        assert!(mean(&e) > 150.0, "{e:?}");
    }
}

#[test]
fn stored_transcriptions_with_the_first_timing_are_moved_into_place() {
    for run in runs() {
        let old = to_lrc(&to_lines(&first_rule(&run.segments)));
        let fixed = retime_v1(&old).expect("moved");
        let l = parse_lyrics_text(&fixed);
        assert!(l.synced);
        let starts: Vec<(String, u32)> = l.timed.iter().filter(|t| !t.text.is_empty()).map(|t| (t.text.clone(), t.start_ms)).collect();
        let e = errors(&run.truth, &starts);
        println!("{} (moved): errors {e:?} ms", run.name);
        assert!(e.iter().all(|x| x.abs() <= TOLERANCE_MS), "{}: {e:?}", run.name);
        // Same words, same lines.
        let before = parse_lyrics_text(&old);
        assert_eq!(before.timed.iter().map(|t| &t.text).collect::<Vec<_>>(), l.timed.iter().map(|t| &t.text).collect::<Vec<_>>());
        for t in &l.timed {
            assert!(t.words.windows(2).all(|w| w[0].start_ms <= w[1].start_ms), "words stay in order");
            if let Some(w) = t.words.first() {
                assert_eq!(w.start_ms, t.start_ms, "a line starts with its first word");
            }
        }
    }
}

#[test]
fn a_word_starts_one_token_before_its_attention_time() {
    let tok = |b: &str, dtw: u32| RawToken { bytes: b.as_bytes().to_vec(), start_ms: 0, end_ms: 0, dtw_ms: Some(dtw) };
    // " hold" ends at 10.40 s, " on" at 10.70 s: "hold" began about 0.22 s before it ended.
    let segs = segments_from_tokens(&[vec![tok(" hold", 10_400), tok(" on", 10_700)], vec![tok(" now", 13_000)]]);
    assert_eq!((segs[0].words[0].start_ms, segs[0].words[0].end_ms), (10_175, 10_400));
    // Never before the word before it.
    assert_eq!(segs[0].words[1].start_ms, 10_512);
    // After a pause the next word's length isn't known: a typical token's.
    assert_eq!(segs[1].words[0].start_ms, 13_000 - 188);
    // A word made of two tokens is timed by its first.
    let segs = segments_from_tokens(&[vec![tok(" sil", 2000), tok("ver", 2200), tok(" bridge", 2500)]]);
    assert_eq!(segs[0].words[0].text, " silver");
    assert_eq!((segs[0].words[0].start_ms, segs[0].words[0].end_ms), (1850, 2200));
    // Without attention times the token's own times are used.
    let plain = RawToken { bytes: b" hey".to_vec(), start_ms: 500, end_ms: 900, dtw_ms: None };
    let segs = segments_from_tokens(&[vec![plain]]);
    assert_eq!((segs[0].words[0].start_ms, segs[0].words[0].end_ms), (500, 900));
}

#[test]
fn moving_old_timing_leaves_gaps_and_plain_lines_alone() {
    let old = "[00:12.34]<00:12.34>Hello <00:12.80>world\n[00:13.70]\n[00:25.00]<00:25.00>Again\n";
    let fixed = retime_v1(old).unwrap();
    assert_eq!(fixed, "[00:11.99]<00:11.99>Hello <00:12.61>world\n[00:13.70]\n[00:24.81]<00:24.81>Again\n");
    assert_eq!(retime_v1("[00:01.00]Plain line\n"), None);
    assert_eq!(retime_v1(""), None);
}
