//! Every tag check rule, on made-up albums, and a clean album that raises nothing.

use std::collections::BTreeSet;

use super::check::{CheckAlbum, CheckFile, Finding, Library, NamePattern, Rule, Severity, check_album, parse_name, report};
use super::fields::Field;

fn file(id: &str, name: &str, pairs: &[(Field, &str)]) -> CheckFile {
    CheckFile {
        track_id: id.into(),
        file_name: name.into(),
        folder: "/srv/music/Demo Artist/Demo Album".into(),
        values: pairs.iter().map(|(f, v)| (*f, v.to_string())).collect(),
        keys: Vec::new(),
        duration_ms: 200_000,
        codec: Some("flac".into()),
        bitrate: Some(900),
        plex_disc: 1,
    }
}

/// A tidy song: album fields, numbers and a title matching its file name.
fn good(n: u32, title: &str) -> CheckFile {
    let num = n.to_string();
    let name = format!("{n:02} - {title}.flac");
    let mut f = file(&format!("t{n}"), &name, &[
        (Field::AlbumArtist, "Demo Artist"),
        (Field::Album, "Demo Album"),
        (Field::Year, "2019"),
        (Field::Artist, "Demo Artist"),
        (Field::Title, title),
        (Field::DiscNumber, "1"),
        (Field::DiscTotal, "1"),
        (Field::TrackTotal, "3"),
    ]);
    f.values.insert(Field::TrackNumber, num);
    f
}

fn album(files: Vec<CheckFile>) -> CheckAlbum {
    CheckAlbum { album_id: "al1".into(), title: "Demo Album".into(), files }
}

fn lib() -> Library {
    Library { can_fix: true, ..Default::default() }
}

fn tidy() -> Vec<CheckFile> {
    vec![good(1, "First Song"), good(2, "Second Song"), good(3, "Third Song")]
}

fn run(files: Vec<CheckFile>) -> Vec<Finding> {
    check_album(&album(files), &lib()).0
}

fn rules(f: &[Finding]) -> Vec<Rule> {
    f.iter().map(|x| x.rule).collect()
}

fn only(f: &[Finding], r: Rule) -> &Finding {
    let all: Vec<&Finding> = f.iter().filter(|x| x.rule == r).collect();
    assert_eq!(all.len(), 1, "{r:?} once in {:?}", rules(f));
    all[0]
}

#[test]
fn a_tidy_album_raises_nothing_and_has_a_style() {
    let (f, style) = check_album(&album(tidy()), &lib());
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(style.unwrap().text, "This album's style: files named \"01 - Title\"");
}

#[test]
fn missing_album_artist_is_an_error() {
    let mut fs = tidy();
    fs[1].values.remove(&Field::AlbumArtist);
    let f = run(fs);
    let x = only(&f, Rule::MissingAlbumArtist);
    assert_eq!(x.severity, Severity::Error);
    assert_eq!(x.track_ids, ["t2"]);
    assert!(x.why.contains("Various Artists"));
    assert_eq!(x.fix.as_ref().unwrap().field, Some(Field::AlbumArtist));
}

#[test]
fn year_album_or_album_artist_differing_within_an_album() {
    for (field, other) in [(Field::Year, "2020"), (Field::Album, "Demo Album (Deluxe)"), (Field::AlbumArtist, "Other Artist")] {
        let mut fs = tidy();
        fs[2].values.insert(field, other.into());
        let f = run(fs);
        assert!(f.iter().any(|x| x.rule == Rule::AlbumTagsDiffer && x.fix.as_ref().unwrap().field == Some(field)), "{field:?}: {:?}", rules(&f));
    }
    // A full date and a year that agree don't count.
    let mut fs = tidy();
    fs[0].values.insert(Field::Year, "2019-05-31".into());
    assert!(!rules(&run(fs)).contains(&Rule::AlbumTagsDiffer));
}

#[test]
fn release_type_missing_its_first_part() {
    let mut fs = tidy();
    for f in &mut fs {
        f.values.insert(Field::ReleaseType, "demo".into());
    }
    assert!(only(&run(fs.clone()), Rule::ReleaseTypeHalf).what.contains("\"demo\""));
    for f in &mut fs {
        f.values.insert(Field::ReleaseType, "album;demo".into());
    }
    assert!(!rules(&run(fs)).contains(&Rule::ReleaseTypeHalf));
}

#[test]
fn a_group_with_one_member() {
    let mut fs = tidy();
    for f in &mut fs {
        f.values.insert(Field::Group, "demo-album".into());
    }
    assert!(rules(&run(fs.clone())).contains(&Rule::LonelyGroup));
    let mut l = lib();
    l.groups.insert("demo-album".into(), BTreeSet::from(["al1".to_string(), "al2".to_string()]));
    assert!(!rules(&check_album(&album(fs), &l).0).contains(&Rule::LonelyGroup));
}

#[test]
fn one_folder_in_two_plex_albums() {
    let mut l = lib();
    l.folders.insert("/srv/music/Demo Artist/Demo Album".into(), BTreeSet::from(["al1".to_string(), "al9".to_string()]));
    l.titles.insert("al9".into(), "Demo Album (Bonus)".into());
    let f = check_album(&album(tidy()), &l).0;
    let x = only(&f, Rule::SplitAlbum);
    assert_eq!(x.severity, Severity::Error);
    assert!(x.what.contains("Demo Album (Bonus)"), "{}", x.what);
}

#[test]
fn unknown_archive_values_and_tags() {
    let mut fs = tidy();
    fs[0].values.insert(Field::ArchiveVersion, "leek".into());
    fs[1].values.insert(Field::Quality, "hq".into());
    fs[1].values.insert(Field::Flags, "fanedit,remaster".into());
    fs[2].values.insert(Field::ArchiveDate, "Summer 2019".into());
    fs[2].keys = vec!["ARCHIVE_VERISON".into(), "TXXX:ARCHIVE_ERA".into()];
    let f = run(fs);
    let whats: Vec<&str> = f.iter().filter(|x| x.rule == Rule::UnknownArchiveValue).map(|x| x.what.as_str()).collect();
    for w in ["\"leek\"", "\"hq\"", "\"remaster\"", "\"Summer 2019\"", "ARCHIVE_VERISON"] {
        assert!(whats.iter().any(|x| x.contains(w)), "{w} in {whats:?}");
    }
    assert!(!whats.iter().any(|x| x.contains("ARCHIVE_ERA")), "a known tag in a TXXX frame is fine");
}

#[test]
fn missing_disc_tags_on_a_multi_disc_album() {
    let mut fs = tidy();
    fs[2].values.insert(Field::DiscNumber, "2".into());
    fs[2].values.insert(Field::TrackNumber, "1".into());
    fs[2].file_name = "2-01 - Third Song.flac".into();
    fs[0].values.remove(&Field::DiscTotal);
    let f = run(fs.clone());
    assert_eq!(only(&f, Rule::DiscTags).track_ids, ["t1"]);
    // A single-disc album without disc tags is fine.
    let mut one = tidy();
    for f in &mut one {
        f.values.remove(&Field::DiscNumber);
        f.values.remove(&Field::DiscTotal);
    }
    assert!(!rules(&run(one)).contains(&Rule::DiscTags));
}

#[test]
fn release_status_missing_where_the_rest_has_one() {
    let mut fs = tidy();
    fs[0].values.insert(Field::Status, "Released".into());
    fs[1].values.insert(Field::Status, "Released".into());
    let f = run(fs);
    let x = only(&f, Rule::StatusMissing);
    assert_eq!(x.track_ids, ["t3"]);
    assert_eq!(x.fix.as_ref().unwrap().field, Some(Field::Status), "in the album's own scheme");
}

#[test]
fn provenance_note_gaps() {
    let mut fs = tidy();
    fs[0].values.insert(Field::VersionNote, "OG file".into());
    fs[1].values.insert(Field::VersionNote, "OG file".into());
    let f = run(fs);
    assert_eq!(only(&f, Rule::NoteMissing).severity, Severity::Info);
}

#[test]
fn artist_separators_and_feat_placement_follow_the_album() {
    let mut fs = tidy();
    fs[0].values.insert(Field::Artist, "Demo Artist, Other Artist".into());
    fs[1].values.insert(Field::Artist, "Demo Artist, Third Artist".into());
    fs[2].values.insert(Field::Artist, "Demo Artist & Other Artist".into());
    let (f, style) = check_album(&album(fs), &lib());
    assert_eq!(only(&f, Rule::ArtistStyle).track_ids, ["t3"]);
    assert!(style.unwrap().text.contains("ARTIST names joined with commas"));

    let mut fs = tidy();
    fs[0].values.insert(Field::Title, "First Song (feat. Other Artist)".into());
    fs[1].values.insert(Field::Title, "Second Song (feat. Third Artist)".into());
    fs[0].file_name = "01 - First Song (feat. Other Artist).flac".into();
    fs[1].file_name = "02 - Second Song (feat. Third Artist).flac".into();
    fs[2].values.insert(Field::Artist, "Demo Artist feat. Other Artist".into());
    let (f, style) = check_album(&album(fs), &lib());
    let x = only(&f, Rule::FeatPlacement);
    assert_eq!(x.track_ids, ["t3"]);
    assert!(style.unwrap().text.contains("features in the title"));
}

#[test]
fn numbering_gaps_and_repeats_per_disc() {
    let mut fs = tidy();
    fs[2].values.insert(Field::TrackNumber, "2".into());
    fs[2].file_name = "02 - Third Song.flac".into();
    let f = run(fs);
    assert!(f.iter().any(|x| x.rule == Rule::NumberGaps && x.what.contains("used twice")), "{:?}", rules(&f));
    let mut fs = tidy();
    fs[2].values.insert(Field::TrackNumber, "5".into());
    fs[2].file_name = "05 - Third Song.flac".into();
    let f = run(fs);
    assert!(f.iter().any(|x| x.rule == Rule::NumberGaps && x.what == "No track 3, 4"), "{f:?}");
}

#[test]
fn the_same_title_twice() {
    let mut fs = tidy();
    fs[2].values.insert(Field::Title, "First Song".into());
    fs[2].file_name = "03 - First Song.flac".into();
    assert_eq!(only(&run(fs), Rule::DuplicateTitle).track_ids, ["t1", "t3"]);
}

#[test]
fn file_names_follow_one_pattern_and_agree_with_the_tags() {
    assert_eq!(parse_name("1-03 - Song.flac"), (NamePattern::DiscTrack, Some(1), Some(3), "Song".into()));
    assert_eq!(parse_name("03 - Song - Part 2.mp3"), (NamePattern::TrackDash, None, Some(3), "Song - Part 2".into()));
    assert_eq!(parse_name("03. Song.m4a"), (NamePattern::TrackDot, None, Some(3), "Song".into()));
    assert_eq!(parse_name("03 Song.ogg"), (NamePattern::TrackSpace, None, Some(3), "Song".into()));
    assert_eq!(parse_name("Song.flac").0, NamePattern::Other);
    assert_eq!(parse_name("2019 Demo.flac").0, NamePattern::Other, "a year isn't a track number");

    let mut fs = tidy();
    fs[2].file_name = "03. Third Song.flac".into();
    assert_eq!(only(&run(fs), Rule::FilenamePattern).track_ids, ["t3"]);

    let mut fs = tidy();
    fs[1].file_name = "05 - Second Song.flac".into();
    assert!(only(&run(fs), Rule::NumberVsFilename).what.contains("track 5 in its name but track 2"));

    let mut fs = tidy();
    fs[1].values.insert(Field::Title, "Second Song (Remix)".into());
    assert_eq!(only(&run(fs), Rule::TitleVsFilename).severity, Severity::Info);
    // What a file system can't hold isn't a difference.
    let mut fs = tidy();
    fs[1].values.insert(Field::Title, "Second: Song/Reprise".into());
    fs[1].file_name = "02 - Second Song - Reprise.flac".into();
    assert!(!rules(&run(fs.clone())).contains(&Rule::TitleVsFilename));
    // Nor is case alone.
    fs[1].values.insert(Field::Title, "SECOND SONG - REPRISE".into());
    assert!(!rules(&run(fs)).contains(&Rule::TitleVsFilename));
}

#[test]
fn stray_ripper_tags_the_album_doesnt_use() {
    let mut fs = tidy();
    for f in &mut fs {
        f.keys = vec!["TITLE".into(), "ARTIST".into()];
    }
    fs[0].keys.push("REPLAYGAIN_TRACK_GAIN".into());
    fs[1].keys.push("TXXX:iTunNORM".to_ascii_uppercase());
    let f = run(fs.clone());
    let s: Vec<&Finding> = f.iter().filter(|x| x.rule == Rule::StrayTags).collect();
    assert_eq!(s.len(), 2);
    // When most of the album has it, it's the album's own way.
    for f in &mut fs {
        f.keys.push("REPLAYGAIN_TRACK_GAIN".into());
    }
    assert_eq!(run(fs).iter().filter(|x| x.rule == Rule::StrayTags).count(), 1);
}

#[test]
fn short_songs_and_low_bitrate_mp3s_are_info() {
    let mut fs = tidy();
    fs[0].duration_ms = 30_000;
    fs[1].codec = Some("mp3".into());
    fs[1].bitrate = Some(128);
    let f = run(fs);
    assert_eq!(only(&f, Rule::ShortTrack).severity, Severity::Info);
    assert_eq!(only(&f, Rule::LowBitrate).severity, Severity::Info);
}

#[test]
fn a_missing_isrc_is_info_only_and_only_for_released_songs() {
    let mut fs = tidy();
    for f in &mut fs {
        f.values.insert(Field::ArchiveVersion, "released".into());
    }
    fs[0].values.insert(Field::Isrc, "USRC17607839".into());
    fs[2].values.insert(Field::ArchiveVersion, "leak".into());
    let f = run(fs);
    let x = only(&f, Rule::MissingIsrc);
    assert_eq!(x.severity, Severity::Info);
    assert_eq!(x.track_ids, ["t2"]);
}

#[test]
fn without_a_music_folder_there_is_no_fix_and_fine_findings_stay_hidden() {
    let mut fs = tidy();
    fs[0].values.remove(&Field::AlbumArtist);
    let l = Library::default();
    let (f, _) = check_album(&album(fs.clone()), &l);
    assert!(f[0].fix.is_none());
    let id = f[0].id.clone();
    let r = report(&[album(fs)], &l, &BTreeSet::from([id]), false);
    assert!(r.findings.is_empty());
    assert_eq!(r.ignored, 1);
}

#[test]
fn the_check_reads_the_files_through_the_editor_and_remembers_fine() {
    use super::editor::{CheckScope, set_ignored};
    use super::testkit::Lib;
    let lib = Lib::new();
    lib.add("t1", "al1", "Demo Album", "Demo Artist/Demo Album/01 - Demo Song.flac", "song.flac", 1, 1, "Demo Song");
    lib.add("t2", "al1", "Demo Album", "Demo Artist/Demo Album/02 - Other Song.flac", "song.flac", 1, 2, "Other Song");
    lib.retag("Demo Artist/Demo Album/02 - Other Song.flac", &[(Field::Title, "Other Song"), (Field::AlbumArtist, "")]);
    let r = lib.editor.check(&CheckScope::Album("al1".into())).unwrap();
    assert!(r.from_files);
    let x = r.findings.iter().find(|x| x.rule == Rule::MissingAlbumArtist).unwrap();
    assert_eq!(x.track_ids, ["t2"]);
    assert!(x.fix.is_some());
    lib.db.write(|c| set_ignored(c, &x.id, true)).unwrap();
    let r = lib.editor.check(&CheckScope::Artist("ar1".into())).unwrap();
    assert!(!r.findings.iter().any(|x| x.rule == Rule::MissingAlbumArtist));
    assert_eq!(r.ignored, 1);
}
