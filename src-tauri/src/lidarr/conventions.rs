//! How an artist's folder is kept, learned from its existing official
//! albums, so a new album added to the library looks like the ones already
//! there: the folder name ("Album (Year)"), file names ("1-01 - Title"),
//! disc tags, and the tag habits (the artist separator, where "(feat. ...)"
//! goes, the album artist, sort tags, genre, the cover file and how big an
//! embedded cover may be, and the provenance scheme: STATUS and VERSION, the
//! ARCHIVE_* tags, or none). ISRCs and MusicBrainz ids on the new files are
//! kept as they are.
//!
//! Pure: the caller reads the files. With fewer than two official albums,
//! or albums that don't agree, the conventions are low confidence and every
//! album is shown to the user before it's added.

use std::collections::BTreeMap;

use serde::Serialize;

/// One existing official album, as read from its folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AlbumSample {
    pub folder_name: String,
    pub title: String,
    pub year: Option<i32>,
    pub files: Vec<FileSample>,
    /// Other files in the folder (cover.jpg...).
    pub other_files: Vec<String>,
}

/// One audio file of an existing album.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileSample {
    /// The file name with its extension.
    pub name: String,
    pub disc: u32,
    pub track: u32,
    /// Neutral tag names (upper case), a name may repeat.
    pub tags: Vec<(String, String)>,
    /// Bytes of the largest embedded picture.
    pub embedded_cover: Option<usize>,
}

impl FileSample {
    fn get(&self, k: &str) -> Option<&str> {
        self.tags.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    fn all(&self, k: &str) -> Vec<&str> {
        self.tags.iter().filter(|(n, _)| n == k).map(|(_, v)| v.as_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Provenance {
    /// STATUS (e.g. "Released") and a VERSION note starting with `prefix`.
    StatusVersion { status: String, prefix: String },
    /// ARCHIVE_VERSION with this value for released files.
    Archive { value: String },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Feat {
    /// "Title (feat. Other)".
    InTitle,
    /// "Artist feat. Other" in the artist tag.
    InArtist,
    /// Not seen: left as the new files have it.
    AsIs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conventions {
    /// Like "{album} ({year})".
    pub folder: String,
    /// Like "{disc}-{track:02} - {title}".
    pub file: String,
    /// DISCNUMBER is written, and with the disc count ("1/2") or not.
    pub disc_tag: Option<DiscTag>,
    /// Joins several artists into one ARTIST value; None: as they are.
    pub artist_separator: Option<String>,
    pub feat: Feat,
    pub album_artist: Option<String>,
    pub artist_sort: Option<String>,
    pub genre: Option<String>,
    /// A cover image file next to the music (cover.jpg).
    pub cover_file: Option<String>,
    /// Embedded covers are kept up to this many bytes; None: none embedded.
    pub embedded_cover_max: Option<usize>,
    pub provenance: Provenance,
    /// Two or more official albums that agree.
    pub confident: bool,
    /// Why not confident, in words.
    pub doubts: Vec<String>,
    pub albums_seen: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiscTag {
    /// "1".
    Number,
    /// "1/2".
    OfTotal,
}

const FOLDERS: [&str; 6] = ["{album} ({year})", "{year} - {album}", "[{year}] {album}", "{album} [{year}]", "({year}) {album}", "{album}"];
const FILES: [&str; 7] = [
    "{disc}-{track:02} - {title}",
    "{disc}-{track:02} {title}",
    "{disc}{track:02} - {title}",
    "{track:02} - {title}",
    "{track:02}. {title}",
    "{track:02} {title}",
    "{track:02}-{title}",
];

/// A name made safe for every file system Plex runs on.
pub fn safe_name(s: &str) -> String {
    let out: String = s.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c }).collect();
    out.trim().trim_end_matches('.').trim().to_string()
}

/// Letters and digits only, lower case: how names are compared.
fn loose(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub fn render_folder(t: &str, album: &str, year: Option<i32>) -> String {
    let y = year.map(|y| y.to_string()).unwrap_or_default();
    let out = t.replace("{album}", &safe_name(album)).replace("{year}", &y);
    // No year: drop the brackets it would have sat in.
    let out = out.replace(" ()", "").replace("() ", "").replace(" []", "").replace("[] ", "");
    let out = if y.is_empty() { out.trim_start_matches(" - ").trim_start_matches("- ").to_string() } else { out };
    safe_name(&out)
}

pub fn render_file(t: &str, disc: u32, track: u32, title: &str) -> String {
    safe_name(&t.replace("{disc}", &disc.to_string()).replace("{track:02}", &format!("{track:02}")).replace("{title}", &safe_name(title)))
}

fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// The value most samples agree on, with how many agree.
fn most<'a>(vals: impl Iterator<Item = &'a str>) -> Option<(String, usize, usize)> {
    let mut m: BTreeMap<String, usize> = BTreeMap::new();
    let mut n = 0;
    for v in vals {
        n += 1;
        *m.entry(v.to_string()).or_default() += 1;
    }
    m.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(v, c)| (v, c, n))
}

/// Learn an artist's conventions from its official albums.
pub fn learn(albums: &[AlbumSample]) -> Conventions {
    let mut doubts = Vec::new();
    // Folder names.
    // Brackets and dashes count here (loose comparison would make
    // "Album (2019)" and "Album [2019]" the same), only case is ignored.
    let folder_fits = |t: &str| albums.iter().filter(|a| render_folder(t, &a.title, a.year).to_lowercase() == safe_name(&a.folder_name).to_lowercase()).count();
    let (folder, f_hits) = FOLDERS.iter().map(|t| (t.to_string(), folder_fits(t))).rev().max_by_key(|(_, n)| *n).unwrap_or_default();
    if f_hits < albums.len() {
        doubts.push(format!("{} of {} album folders follow one pattern", f_hits, albums.len()));
    }
    // File names, by the title tag.
    let files: Vec<&FileSample> = albums.iter().flat_map(|a| a.files.iter()).collect();
    let file_fits = |t: &str| files.iter().filter(|f| f.get("TITLE").is_some_and(|title| loose(&render_file(t, f.disc, f.track, title)) == loose(stem(&f.name)))).count();
    let (file, n_hits) = FILES.iter().map(|t| (t.to_string(), file_fits(t))).rev().max_by_key(|(_, n)| *n).unwrap_or_default();
    if n_hits < files.len() {
        doubts.push(format!("{} of {} file names follow one pattern", n_hits, files.len()));
    }
    // Disc tags.
    let discs: Vec<&str> = files.iter().filter_map(|f| f.get("DISCNUMBER")).collect();
    let disc_tag = (discs.len() * 2 > files.len()).then(|| if discs.iter().filter(|d| d.contains('/')).count() * 2 > discs.len() { DiscTag::OfTotal } else { DiscTag::Number });
    // Artist separator: among files with one ARTIST value naming several.
    let mut seps: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &files {
        for v in f.all("ARTIST") {
            for s in ["; ", " / ", " & ", ", "] {
                if v.contains(s) {
                    *seps.entry(s).or_default() += 1;
                }
            }
        }
    }
    let artist_separator = seps.into_iter().max_by_key(|(_, n)| *n).map(|(s, _)| s.to_string());
    // Where "feat." goes.
    let in_title = files.iter().filter(|f| f.get("TITLE").is_some_and(|t| t.to_lowercase().contains("(feat."))).count();
    let in_artist = files.iter().filter(|f| f.all("ARTIST").iter().any(|a| a.to_lowercase().contains(" feat."))).count();
    let feat = if in_title > in_artist { Feat::InTitle } else if in_artist > 0 { Feat::InArtist } else { Feat::AsIs };
    let half = |c: usize, n: usize| n > 0 && c * 2 > n;
    let album_artist = most(files.iter().filter_map(|f| f.get("ALBUMARTIST"))).filter(|(_, c, _)| half(*c, files.len())).map(|(v, _, _)| v);
    let artist_sort = most(files.iter().filter_map(|f| f.get("ALBUMARTISTSORT").or(f.get("ARTISTSORT")))).filter(|(_, c, _)| half(*c, files.len())).map(|(v, _, _)| v);
    let genre = most(files.iter().filter_map(|f| f.get("GENRE"))).filter(|(_, c, _)| half(*c, files.len())).map(|(v, _, _)| v);
    // The cover file next to the music.
    let cover_file = most(albums.iter().flat_map(|a| {
        a.other_files.iter().filter(|n| {
            let l = n.to_lowercase();
            (l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png")) && ["cover", "folder", "front"].iter().any(|c| l.starts_with(c))
        })
    }).map(String::as_str))
    .filter(|(_, c, _)| half(*c, albums.len()))
    .map(|(v, _, _)| v);
    let embedded: Vec<usize> = files.iter().filter_map(|f| f.embedded_cover).collect();
    let embedded_cover_max = half(embedded.len(), files.len()).then(|| embedded.iter().copied().max().unwrap_or(0));
    // Provenance.
    let status = most(files.iter().filter_map(|f| f.get("STATUS"))).filter(|(_, c, _)| half(*c, files.len()));
    let archive = most(files.iter().filter_map(|f| f.get("ARCHIVE_VERSION"))).filter(|(_, c, _)| half(*c, files.len()));
    let provenance = match (status, archive) {
        (Some((s, _, _)), _) => {
            let prefix = most(files.iter().filter_map(|f| f.get("VERSION")).map(|v| v.split(" (").next().unwrap_or(v).trim()))
                .map(|(v, _, _)| v)
                .unwrap_or_default();
            Provenance::StatusVersion { status: s, prefix }
        }
        (None, Some((v, _, _))) => Provenance::Archive { value: v },
        _ => Provenance::None,
    };
    if albums.len() < 2 {
        doubts.push(format!("only {} official album to learn from", albums.len()));
    }
    Conventions {
        folder,
        file,
        disc_tag,
        artist_separator,
        feat,
        album_artist,
        artist_sort,
        genre,
        cover_file,
        embedded_cover_max,
        provenance,
        confident: doubts.is_empty(),
        doubts,
        albums_seen: albums.len(),
    }
}

/// One file of a new album, as Lidarr imported it into staging.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewFile {
    /// Lidarr's path.
    pub src: String,
    pub ext: String,
    pub disc: u32,
    pub track: u32,
    pub discs: u32,
    pub title: String,
    pub tags: Vec<(String, String)>,
}

/// One planned file: its new name and every tag Platter sets on it.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    /// Lidarr's path in the staging folder.
    pub src: String,
    pub name: String,
    /// Tags set (Some) or removed (None), in order.
    pub edits: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedAlbum {
    pub folder: String,
    pub files: Vec<PlannedFile>,
}

/// Split "Artist feat. Other" into ("Artist", Some("Other")).
fn split_feat(a: &str) -> (String, Option<String>) {
    let l = a.to_lowercase();
    for pat in [" feat. ", " ft. ", " featuring "] {
        if let Some(i) = l.find(pat) {
            return (a[..i].trim().to_string(), Some(a[i + pat.len()..].trim().to_string()));
        }
    }
    (a.to_string(), None)
}

/// The new album's folder, file names and tags under `c`. `note` is the
/// provenance note's detail ("Lidarr, FLAC, 2026-10-16").
pub fn apply(c: &Conventions, album: &str, year: Option<i32>, files: &[NewFile], note: &str) -> PlannedAlbum {
    let folder = render_folder(&c.folder, album, year);
    let mut out = Vec::new();
    for f in files {
        let mut title = f.title.clone();
        let mut edits: Vec<(String, Option<String>)> = Vec::new();
        let artists: Vec<String> = f.tags.iter().filter(|(k, _)| k == "ARTIST").map(|(_, v)| v.clone()).collect();
        let mut artist = match (&c.artist_separator, artists.len()) {
            (Some(sep), n) if n > 1 => Some(artists.join(sep)),
            (_, 1) => Some(artists[0].clone()),
            _ => None,
        };
        if c.feat == Feat::InTitle
            && let Some(a) = artist.clone()
        {
            let (main, guest) = split_feat(&a);
            if let Some(g) = guest
                && !title.to_lowercase().contains("feat.")
            {
                title = format!("{title} (feat. {g})");
                artist = Some(main);
            }
        }
        if let Some(a) = artist.filter(|a| artists.len() != 1 || *a != artists[0]) {
            edits.push(("ARTIST".into(), Some(a)));
        }
        if title != f.title {
            edits.push(("TITLE".into(), Some(title.clone())));
        }
        if let Some(aa) = &c.album_artist {
            edits.push(("ALBUMARTIST".into(), Some(aa.clone())));
        }
        if let Some(s) = &c.artist_sort {
            edits.push(("ALBUMARTISTSORT".into(), Some(s.clone())));
        }
        if let Some(g) = &c.genre {
            edits.push(("GENRE".into(), Some(g.clone())));
        }
        match c.disc_tag {
            Some(DiscTag::Number) => edits.push(("DISCNUMBER".into(), Some(f.disc.max(1).to_string()))),
            Some(DiscTag::OfTotal) => edits.push(("DISCNUMBER".into(), Some(format!("{}/{}", f.disc.max(1), f.discs.max(1))))),
            None => {}
        }
        match &c.provenance {
            Provenance::StatusVersion { status, prefix } => {
                edits.push(("STATUS".into(), Some(status.clone())));
                let p = if prefix.trim().is_empty() { "Official release" } else { prefix.as_str() };
                edits.push(("VERSION".into(), Some(format!("{p} ({note})"))));
            }
            Provenance::Archive { value } => edits.push(("ARCHIVE_VERSION".into(), Some(value.clone()))),
            Provenance::None => {}
        }
        // Keep only real changes.
        edits.retain(|(k, v)| v.as_deref() != f.tags.iter().find(|(n, _)| n == k).map(|(_, x)| x.as_str()) || f.tags.iter().filter(|(n, _)| n == k).count() > 1);
        let ext = f.ext.trim_start_matches('.').to_ascii_lowercase();
        out.push(PlannedFile { src: f.src.clone(), name: format!("{}.{ext}", render_file(&c.file, f.disc.max(1), f.track, &title)), edits });
    }
    PlannedAlbum { folder, files: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, disc: u32, track: u32, tags: &[(&str, &str)]) -> FileSample {
        FileSample { name: name.into(), disc, track, tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), embedded_cover: Some(120_000) }
    }

    fn tags(title: &str, extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, String)> {
        let mut v = vec![("TITLE", title.to_string())];
        v.extend(extra.iter().map(|(k, x)| (*k, x.to_string())));
        v
    }

    fn sample(folder: &str, title: &str, year: i32, songs: &[(u32, u32, &str)], extra: &[(&'static str, &'static str)]) -> AlbumSample {
        AlbumSample {
            folder_name: folder.into(),
            title: title.into(),
            year: Some(year),
            files: songs
                .iter()
                .map(|(d, t, name)| {
                    let tv = tags(name, extra);
                    let pairs: Vec<(&str, &str)> = tv.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    file(&format!("{d}-{t:02} - {name}.flac"), *d, *t, &pairs)
                })
                .collect(),
            other_files: vec!["cover.jpg".into()],
        }
    }

    const EXTRA: &[(&str, &str)] = &[
        ("ALBUMARTIST", "Northlight"),
        ("ALBUMARTISTSORT", "Northlight"),
        ("GENRE", "Indie"),
        ("DISCNUMBER", "1"),
        ("STATUS", "Released"),
        ("VERSION", "Official release (CD, 2019)"),
        ("ARTIST", "Northlight"),
    ];

    fn two() -> Vec<AlbumSample> {
        vec![
            sample("First Light (2019)", "First Light", 2019, &[(1, 1, "Dawn"), (1, 2, "Noon: Part 1")], EXTRA),
            sample("Second Wind (2021)", "Second Wind", 2021, &[(1, 1, "Rise"), (2, 1, "Fall")], EXTRA),
        ]
    }

    #[test]
    fn folder_file_and_tag_habits_are_learned() {
        let c = learn(&two());
        assert_eq!(c.folder, "{album} ({year})");
        assert_eq!(c.file, "{disc}-{track:02} - {title}");
        assert_eq!(c.disc_tag, Some(DiscTag::Number));
        assert_eq!(c.album_artist.as_deref(), Some("Northlight"));
        assert_eq!(c.genre.as_deref(), Some("Indie"));
        assert_eq!(c.cover_file.as_deref(), Some("cover.jpg"));
        assert_eq!(c.embedded_cover_max, Some(120_000));
        assert_eq!(c.provenance, Provenance::StatusVersion { status: "Released".into(), prefix: "Official release".into() });
        assert!(c.confident, "{:?}", c.doubts);
    }

    #[test]
    fn one_album_or_disagreeing_albums_are_low_confidence() {
        let c = learn(&two()[..1]);
        assert!(!c.confident);
        assert!(c.doubts.iter().any(|d| d.contains("only 1 official album")));
        let mut odd = two();
        odd[1].folder_name = "2021 - Second Wind".into();
        let c = learn(&odd);
        assert!(!c.confident);
        assert!(c.doubts[0].contains("1 of 2 album folders"), "{:?}", c.doubts);
    }

    #[test]
    fn a_new_album_gets_the_names_and_tags_of_the_old_ones_and_keeps_its_ids() {
        let c = learn(&two());
        let files = vec![
            NewFile {
                src: "/data/staging/music/Northlight/Third Way (2026)/01 - Arrive.flac".into(),
                ext: "flac".into(),
                disc: 1,
                track: 1,
                discs: 1,
                title: "Arrive".into(),
                tags: vec![
                    ("TITLE".into(), "Arrive".into()),
                    ("ARTIST".into(), "Northlight feat. Harbor Sound".into()),
                    ("ISRC".into(), "XXA012600001".into()),
                    ("MUSICBRAINZ_TRACKID".into(), "abc".into()),
                ],
            },
            NewFile { src: "/s/02.flac".into(), ext: "FLAC".into(), disc: 1, track: 2, discs: 1, title: "Why? / Why Not".into(), tags: vec![("ARTIST".into(), "Northlight".into())] },
        ];
        let mut c2 = c.clone();
        c2.feat = Feat::InTitle;
        let p = apply(&c2, "Third Way: Live?", Some(2026), &files, "Lidarr, FLAC, 2026-10-16");
        assert_eq!(p.folder, "Third Way_ Live_ (2026)");
        assert_eq!(p.files[0].name, "1-01 - Arrive (feat. Harbor Sound).flac");
        assert_eq!(p.files[1].name, "1-02 - Why_ _ Why Not.flac");
        let e = &p.files[0].edits;
        let get = |k: &str| e.iter().find(|(n, _)| n == k).and_then(|(_, v)| v.clone());
        assert_eq!(get("ARTIST").as_deref(), Some("Northlight"));
        assert_eq!(get("TITLE").as_deref(), Some("Arrive (feat. Harbor Sound)"));
        assert_eq!(get("STATUS").as_deref(), Some("Released"));
        assert_eq!(get("VERSION").as_deref(), Some("Official release (Lidarr, FLAC, 2026-10-16)"));
        assert_eq!(get("DISCNUMBER").as_deref(), Some("1"));
        assert!(!e.iter().any(|(k, _)| k == "ISRC" || k.starts_with("MUSICBRAINZ")), "ids are never touched");
        // ARTIST already right: not rewritten.
        assert!(!p.files[1].edits.iter().any(|(k, _)| k == "ARTIST"));
    }

    #[test]
    fn archive_tags_and_multi_artist_values() {
        let extra: &[(&str, &str)] = &[("ARCHIVE_VERSION", "released"), ("ARTIST", "Northlight; Glasshouse")];
        let a = vec![
            sample("First Light", "First Light", 2019, &[(1, 1, "Dawn")], extra),
            sample("Second Wind", "Second Wind", 2021, &[(1, 1, "Rise")], extra),
        ];
        let c = learn(&a);
        assert_eq!(c.folder, "{album}");
        assert_eq!(c.provenance, Provenance::Archive { value: "released".into() });
        assert_eq!(c.artist_separator.as_deref(), Some("; "));
        let f = NewFile { src: "/s/1.flac".into(), ext: "flac".into(), disc: 1, track: 1, discs: 1, title: "Arrive".into(), tags: vec![("ARTIST".into(), "Northlight".into()), ("ARTIST".into(), "Glasshouse".into())] };
        let p = apply(&c, "Third Way", None, &[f], "x");
        assert!(p.files[0].edits.contains(&("ARTIST".into(), Some("Northlight; Glasshouse".into()))));
        assert!(p.files[0].edits.contains(&("ARCHIVE_VERSION".into(), Some("released".into()))));
        assert_eq!(render_folder("{album} ({year})", "Third Way", None), "Third Way");
    }
}
