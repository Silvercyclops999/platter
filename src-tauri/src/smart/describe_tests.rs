//! Describe a playlist: a table of descriptions and the rules they make,
//! other readings, words not understood, library names with folding, date
//! ranges in local time, and nothing sent anywhere.

use super::describe::{Described, Vocabulary, describe, local_day};
use super::eval::{self, Row};
use super::rules::*;

const TODAY: Day = (2026, 10, 15);

fn vocab() -> Vocabulary {
    let mut v = Vocabulary::default();
    for g in ["Rock", "Jazz", "Hip-Hop", "R&B", "Electronic", "Country", "Ambient", "Pop"] {
        v.add(Field::Genre, g);
    }
    for s in ["Shoegaze", "Ambient", "Trip Hop"] {
        v.add(Field::Style, s);
    }
    for m in ["Chill", "Late Night", "Upbeat", "Sad", "Energetic", "Mellow"] {
        v.add(Field::Mood, m);
    }
    for a in ["Kanye West", "Drake", "Beyoncé", "Sigur Rós", "Michael Jackson", "Michael Bublé", "The Beatles"] {
        v.add(Field::Artist, a);
    }
    for a in ["Yandhi", "Jazz", "Abbey Road"] {
        v.add(Field::Album, a);
    }
    for e in ["Yandhi", "Graduation"] {
        v.add(Field::Era, e);
    }
    for x in ["Demo", "Remaster", "Live"] {
        v.add(Field::Version, x);
    }
    v
}

fn show(r: &Rule) -> String {
    let f = serde_json::to_value(r.field).unwrap();
    let o = serde_json::to_value(r.op).unwrap();
    format!("{} {} {}", f.as_str().unwrap(), o.as_str().unwrap(), r.value)
}

fn run(text: &str) -> Described {
    describe(text, &vocab(), TODAY)
}

fn rules_of(d: &Described) -> Vec<String> {
    d.rules.rules.iter().map(show).collect()
}

/// (description, rules, words not understood)
const TABLE: &[(&str, &[&str], &[&str])] = &[
    // The five from the brief.
    ("chill late-night stuff from 2016 I haven't played lately", &["mood is Chill", "mood is Late Night", "year is 2016", "lastPlayed notInLast 3m"], &[]),
    ("only unreleased Kanye-style leaks from the Yandhi era", &["unreleased is true", "artist is Kanye West", "era is Yandhi"], &[]),
    ("upbeat songs under 3 minutes, lossless only", &["mood is Upbeat", "duration lt 3", "lossless is true"], &[]),
    ("songs I loved last summer", &["liked is true", "played between 2026-06-01..2026-08-31"], &[]),
    ("albums from the 90s I never finished", &["releaseType is albums", "decade is 1990", "finished is false"], &[]),
    // Library names, folded.
    ("rock", &["genre is Rock"], &[]),
    ("ROCK", &["genre is Rock"], &[]),
    ("hip hop", &["genre is Hip-Hop"], &[]),
    ("hiphop", &["genre is Hip-Hop"], &[]),
    ("r&b", &["genre is R&B"], &[]),
    ("beyonce", &["artist is Beyoncé"], &[]),
    ("BEYONCÉ", &["artist is Beyoncé"], &[]),
    ("sigur-ros", &["artist is Sigur Rós"], &[]),
    ("drake", &["artist is Drake"], &[]),
    ("not drake", &["artist isNot Drake"], &[]),
    ("no country", &["genre isNot Country"], &[]),
    ("shoegaze", &["style is Shoegaze"], &[]),
    ("trip-hop", &["style is Trip Hop"], &[]),
    ("the beatles", &["artist is The Beatles"], &[]),
    ("beatles", &["artist is The Beatles"], &[]),
    ("Abbey Road", &["album is Abbey Road"], &[]),
    ("from the Graduation era", &["era is Graduation"], &[]),
    ("sad songs", &["mood is Sad"], &[]),
    ("mellow", &["mood is Mellow"], &[]),
    ("energetic", &["mood is Energetic"], &[]),
    ("happy", &["mood is Upbeat"], &[]),
    ("edm", &["genre is Electronic"], &[]),
    // Release years and decades.
    ("2016", &["year is 2016"], &[]),
    ("before 2000", &["year lt 2000"], &[]),
    ("after 2010", &["year gt 2010"], &[]),
    ("since 2015", &["year gt 2014"], &[]),
    ("between 1995 and 1999", &["year gt 1994", "year lt 2000"], &[]),
    ("2010-2015", &["year gt 2009", "year lt 2016"], &[]),
    ("Hip-Hop from 2010 to 2015", &["genre is Hip-Hop", "year gt 2009", "year lt 2016"], &[]),
    ("the 90s", &["decade is 1990"], &[]),
    ("1980s", &["decade is 1980"], &[]),
    ("eighties", &["decade is 1980"], &[]),
    ("early 2000s", &["year gt 1999", "year lt 2004"], &[]),
    ("late 90s", &["year gt 1995", "year lt 2000"], &[]),
    ("from last year", &["year is 2025"], &[]),
    // Plays and when.
    ("never played", &["playCount is 0"], &[]),
    ("unplayed", &["playCount is 0"], &[]),
    ("haven't played in a while", &["lastPlayed notInLast 6m"], &[]),
    ("not played in the last 2 weeks", &["lastPlayed notInLast 2w"], &[]),
    ("played recently", &["lastPlayed inLast 3m"], &[]),
    ("played in the last month", &["lastPlayed inLast 1m"], &[]),
    ("in the last 30 days", &["lastPlayed inLast 30d"], &[]),
    ("played more than 10 times", &["playCount gt 10"], &[]),
    ("10+ plays", &["playCount gt 9"], &[]),
    ("rarely played", &["playCount lt 3"], &[]),
    ("played a lot", &["playCount gt 9"], &[]),
    ("played in 2019", &["played between 2019-01-01..2019-12-31"], &[]),
    ("played in june", &["played between 2026-06-01..2026-06-30"], &[]),
    ("this year", &["played between 2026-01-01..2026-10-15"], &[]),
    ("last year", &["played between 2025-01-01..2025-12-31"], &[]),
    ("last week", &["played between 2026-10-05..2026-10-11"], &[]),
    ("last winter", &["played between 2025-12-01..2026-02-28"], &[]),
    ("summer of 2019", &["played between 2019-06-01..2019-08-31"], &[]),
    ("recently added", &["addedAt inLast 30d"], &[]),
    ("added last month", &["addedAt between 2026-09-01..2026-09-30"], &[]),
    ("added in 2020", &["addedAt between 2020-01-01..2020-12-31"], &[]),
    // Skips and likes.
    ("skipped", &["skips gt 0"], &[]),
    ("songs I never skip", &["skips is 0"], &[]),
    ("skipped more than 3 times", &["skips gt 3"], &[]),
    ("liked", &["liked is true"], &[]),
    ("songs I like", &["liked is true"], &[]),
    ("songs I don't like", &["liked is false"], &[]),
    ("5 stars", &["rating is 5"], &[]),
    ("rated 4 stars", &["rating gt 3"], &[]),
    ("top rated", &["rating gt 3"], &[]),
    // Length and quality.
    ("under 3 minutes", &["duration lt 3"], &[]),
    ("longer than 6 min", &["duration gt 6"], &[]),
    ("over 5 minutes", &["duration gt 5"], &[]),
    ("under 90 seconds", &["duration lt 1.5"], &[]),
    ("5 minutes or longer", &["duration gt 5"], &[]),
    ("short songs", &["duration lt 3"], &[]),
    ("lossless", &["lossless is true"], &[]),
    ("flac", &["lossless is true"], &[]),
    ("hi-res", &["lossless is true"], &[]),
    ("mp3", &["lossless is false"], &[]),
    ("explicit", &["explicit is true"], &[]),
    ("not explicit", &["explicit is false"], &[]),
    ("family friendly", &["explicit is false"], &[]),
    // Released, release types and versions.
    ("unreleased", &["unreleased is true"], &[]),
    ("released", &["unreleased is false"], &[]),
    ("officially released", &["unreleased is false"], &[]),
    ("not released", &["unreleased is true"], &[]),
    ("without leaks", &["unreleased is false"], &[]),
    ("live", &["releaseType is live"], &[]),
    ("singles and EPs", &["releaseType is singlesEps"], &[]),
    ("no remixes", &["releaseType isNot remixes"], &[]),
    ("demos", &["releaseType is demos"], &[]),
    ("remastered", &["version contains remaster"], &[]),
    ("acoustic", &["version contains acoustic"], &[]),
    ("radio edit", &["version contains radio edit"], &[]),
    // Several at once.
    ("never skipped and lossless", &["skips is 0", "lossless is true"], &[]),
    ("albums I never finished", &["releaseType is albums", "finished is false"], &[]),
    ("unfinished albums", &["finished is false", "releaseType is albums"], &[]),
    // Words not understood are listed, not guessed.
    ("blorp", &[], &["blorp"]),
    ("chill zxq fnord from 2016", &["mood is Chill", "year is 2016"], &["zxq fnord"]),
    ("dreamy", &[], &["dreamy"]),
    ("rock with kazoo solos", &["genre is Rock"], &["kazoo solos"]),
    ("songs about rain", &[], &["rain"]),
];

#[test]
fn the_table_of_descriptions_makes_these_rules() {
    assert!(TABLE.len() >= 60, "{} rows", TABLE.len());
    let mut wrong = Vec::new();
    for (text, rules, unknown) in TABLE {
        let d = run(text);
        let got = rules_of(&d);
        if got != *rules || d.unknown != *unknown {
            wrong.push(format!("{text:?}: rules {got:?} (want {rules:?}), unknown {:?} (want {unknown:?})", d.unknown));
        }
        // Every rule made checks out.
        d.rules.check().unwrap_or_else(|e| panic!("{text:?}: {e:?}"));
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

#[test]
fn limits_sorts_and_match_any() {
    let d = run("50 songs");
    assert_eq!(d.rules.limit, Some(Limit { unit: LimitUnit::Songs, value: 50 }));
    let d = run("2 hours of jazz");
    assert_eq!(d.rules.limit, Some(Limit { unit: LimitUnit::Hours, value: 2 }));
    assert_eq!(rules_of(&d), ["genre is Jazz"]);
    let d = run("an hour of upbeat music");
    assert_eq!(d.rules.limit, Some(Limit { unit: LimitUnit::Hours, value: 1 }));
    assert_eq!(rules_of(&d), ["mood is Upbeat"]);
    let d = run("top 20");
    assert_eq!((d.rules.sort, d.rules.limit.map(|l| l.value)), (SortBy::MostPlayed, Some(20)));
    assert_eq!(run("most played").rules.sort, SortBy::MostPlayed);
    assert_eq!(run("random rock").rules.sort, SortBy::Random);
    // Two of one field mean either.
    let d = run("rock or jazz");
    assert!(!d.rules.match_all);
    assert_eq!(rules_of(&d), ["genre is Rock", "genre is Jazz"]);
    // Mixed with other rules, every rule still has to match, and it says so.
    let d = run("rock and jazz, lossless");
    assert!(d.rules.match_all);
    assert_eq!(d.notes.len(), 1, "{:?}", d.notes);
    assert!(d.notes[0].contains("Rock and Jazz"), "{}", d.notes[0]);
}

#[test]
fn ambiguous_words_take_the_first_reading_and_offer_the_others() {
    let one = |text: &str| {
        let d = run(text);
        assert_eq!(d.choices.len(), 1, "{text}: {:?}", d.choices);
        let c = d.choices[0].clone();
        (show(&d.rules.rules[c.index]), c.others.iter().map(show).collect::<Vec<_>>(), c.text)
    };
    assert_eq!(one("ambient"), ("genre is Ambient".into(), vec!["style is Ambient".to_string()], "ambient".into()));
    assert_eq!(one("jazz"), ("genre is Jazz".into(), vec!["album is Jazz".to_string()], "jazz".into()));
    assert_eq!(one("Yandhi"), ("album is Yandhi".into(), vec!["era is Yandhi".to_string()], "Yandhi".into()));
    assert_eq!(one("demos"), ("releaseType is demos".into(), vec!["version contains Demo".to_string()], "demos".into()));
    assert_eq!(one("live"), ("releaseType is live".into(), vec!["version contains Live".to_string()], "live".into()));
    assert_eq!(one("20s"), ("decade is 2020".into(), vec!["decade is 1920".to_string()], "20s".into()));
    assert_eq!(one("michael"), ("artist is Michael Bublé".into(), vec!["artist is Michael Jackson".to_string()], "michael".into()));
    // "not" carries into the other readings.
    assert_eq!(one("no ambient").1, ["style isNot Ambient"]);
    // An era named outright is the era, not the album.
    assert!(run("the Yandhi era").choices.is_empty());
}

#[test]
fn library_names_come_from_the_cache_rows() {
    let row = Row {
        genres: vec!["Hip-Hop".into()],
        moods: vec!["Late Night".into()],
        artist: "Kanye West".into(),
        track_artist: Some("Kid Cudi".into()),
        album: "808s & Heartbreak".into(),
        era: Some("Yandhi".into()),
        version: Some("Remaster".into()),
        ..Default::default()
    };
    let v = Vocabulary::from_rows(&[row]);
    let d = describe("late night hiphop by kid cudi from the yandhi era, 808s and heartbreak", &v, TODAY);
    assert_eq!(rules_of(&d), ["mood is Late Night", "genre is Hip-Hop", "artist is Kid Cudi", "era is Yandhi", "album is 808s & Heartbreak"]);
    assert!(d.unknown.is_empty(), "{:?}", d.unknown);
    // Nothing in the library: mood words aren't guessed.
    let empty = Vocabulary::default();
    let d = describe("chill rock", &empty, TODAY);
    assert!(d.rules.rules.is_empty());
    assert_eq!(d.unknown, ["chill rock"]);
}

#[test]
fn seasons_and_weeks_follow_today() {
    // In the middle of summer, last summer is last year's.
    let d = describe("songs I loved last summer", &vocab(), (2026, 7, 10));
    assert_eq!(rules_of(&d)[1], "played between 2025-06-01..2025-08-31");
    // "this summer" in July stops at today.
    let d = describe("this summer", &vocab(), (2026, 7, 10));
    assert_eq!(rules_of(&d), ["played between 2026-06-01..2026-07-10"]);
    // January: last month is December, last winter is the one before.
    let d = describe("last month", &vocab(), (2026, 1, 20));
    assert_eq!(rules_of(&d), ["played between 2025-12-01..2025-12-31"]);
    let d = describe("last winter", &vocab(), (2026, 1, 20));
    assert_eq!(rules_of(&d), ["played between 2024-12-01..2025-02-28"]);
    // A leap February.
    let d = describe("last winter", &vocab(), (2024, 6, 1));
    assert_eq!(rules_of(&d), ["played between 2023-12-01..2024-02-29"]);
    // The title is the description, tidied.
    assert_eq!(run("songs I loved last summer.").title, "Songs I loved last summer");
}

const DAY: i64 = 86_400;

fn played_at(t: i64) -> Row {
    Row { id: "1".into(), plays_at: vec![t], ..Default::default() }
}

#[test]
fn date_ranges_are_local_days() {
    let june = rule_between("2026-06-01..2026-06-30");
    let first = crate::stats::days_from_civil(2026, 6, 1) * DAY;
    // 00:30 on 1 June in UTC+10 is 14:30 on 31 May in UTC.
    let row = played_at(first - 10 * 3600 + 30 * 60);
    assert!(eval::matches_tz(&june, &row, 0, 10.0, 600));
    assert!(!eval::matches_tz(&june, &row, 0, 10.0, 0));
    // 23:30 on 30 June in UTC-5 is 04:30 on 1 July in UTC.
    let last = crate::stats::days_from_civil(2026, 6, 30) * DAY;
    let row = played_at(last + 23 * 3600 + 30 * 60 + 5 * 3600);
    assert!(eval::matches_tz(&june, &row, 0, 10.0, -300));
    assert!(!eval::matches_tz(&june, &row, 0, 10.0, 0));
    // Today is the local day too.
    let now = crate::stats::days_from_civil(2026, 10, 15) * DAY + 2 * 3600;
    assert_eq!(local_day(now, -300), (2026, 10, 14));
    assert_eq!(local_day(now, 0), (2026, 10, 15));
    // A backwards range doesn't check out.
    assert!(rule_between("2026-08-31..2026-06-01").check().is_err());
    assert!(rule_between("summer").check().is_err());
}

fn rule_between(v: &str) -> Rule {
    Rule { field: Field::Played, op: Op::Between, value: v.into(), plex_field: None }
}

#[test]
fn new_fields_evaluate_and_stay_on_this_computer() {
    let mut a = Row { id: "1".into(), album_id: "al".into(), plays: 3, lossless: true, skips: 0, era: Some("Yandhi".into()), unreleased: true, ..Default::default() };
    let mut b = Row { id: "2".into(), album_id: "al".into(), plays: 0, skips: 4, ..Default::default() };
    let c = Row { id: "3".into(), album_id: "full".into(), plays: 1, ..Default::default() };
    a.plays_at = vec![100];
    b.plays_at = vec![];
    let mut rows = vec![a, b, c];
    eval::mark_finished(&mut rows);
    let ids = |r: Rule| {
        let rules = SmartRules { rules: vec![r.clone()], ..Default::default() };
        assert!(!rules.plex_gaps().is_empty(), "{r:?} is filled here");
        eval::evaluate(&rows, &rules, 1_000, 10.0, 0).into_iter().map(|r| r.id.clone()).collect::<Vec<_>>()
    };
    let r = |f, op, v: &str| Rule { field: f, op, value: v.into(), plex_field: None };
    assert_eq!(ids(r(Field::Lossless, Op::Is, "true")), ["1"]);
    assert_eq!(ids(r(Field::Unreleased, Op::Is, "true")), ["1"]);
    assert_eq!(ids(r(Field::Era, Op::Is, "yandhi")), ["1"]);
    assert_eq!(ids(r(Field::Skips, Op::Gt, "0")), ["2"]);
    assert_eq!(ids(r(Field::Finished, Op::Is, "false")), ["1", "2"]);
    assert_eq!(ids(r(Field::Finished, Op::Is, "true")), ["3"]);
    assert_eq!(ids(r(Field::Played, Op::InLast, "1d")), ["1"]);
}

#[tokio::test]
async fn describing_sends_nothing_anywhere() {
    let r = super::tests::rig_for_describe();
    r.smart.set_tz(0);
    let d = r.smart.describe("rock from 1999 I haven't played lately, lossless").await.unwrap();
    assert_eq!(rules_of(&d), ["genre is Rock", "year is 1999", "lastPlayed notInLast 3m", "lossless is true"]);
    assert!(r.web.log.lock().is_empty(), "no request made");
    assert!(r.plex.created.lock().is_empty() && r.plex.updated.lock().is_empty());
    assert!(r.smart.describe("  ").await.is_err());
    // The live count works on what it made (the rig's files are FLAC).
    let p = r.smart.preview(d.rules, vec![]).await.unwrap();
    assert_eq!(p.count, 3);
    assert!(r.web.log.lock().is_empty());
}
