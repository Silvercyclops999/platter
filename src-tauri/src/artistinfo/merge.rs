//! What the artist page shows: Plex first, the outside sources filling gaps.
//! Pure, so the order rules are easy to test.

use std::collections::HashMap;

use super::listenbrainz::LbStats;
use super::mbdetails::MbArtist;
use super::{
    ArtistInfo, AudioDbData, BioLink, BioSource, Credit, FanartData, InfoLink, LastfmData, Member, Related, WikiData,
    ext_art_path, fold_name,
};

/// A Plex summary at least this long is kept even when another source has more.
pub const PLEX_BIO_ENOUGH: usize = 300;

const WIKIPEDIA_LICENCE: &str = "CC BY-SA 4.0";
const WIKIPEDIA_LICENCE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";
const LASTFM_LICENCE: &str = "CC BY-SA";
const LASTFM_LICENCE_URL: &str = "https://creativecommons.org/licenses/by-sa/3.0/";

/// The Plex side of an artist.
#[derive(Debug, Clone, Default)]
pub struct PlexArtist {
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub thumb: Option<String>,
    pub art: Option<String>,
}

/// Cached source data, already left out for sources that are off.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    pub wiki: Option<WikiData>,
    pub lastfm: Option<LastfmData>,
    pub fanart: Option<FanartData>,
    pub audiodb: Option<AudioDbData>,
    pub musicbrainz: Option<MbArtist>,
    pub lb_stats: Option<LbStats>,
}

impl Sources {
    /// Names the page may link to library artists: members and relations.
    pub fn people(&self) -> Vec<String> {
        let mut out: Vec<String> = self.wiki.as_ref().map(|w| w.members.clone()).unwrap_or_default();
        if let Some(mb) = &self.musicbrainz {
            out.extend(mb.members.iter().map(|m| m.name.clone()));
            out.extend(mb.related.iter().map(|r| r.name.clone()));
        }
        out
    }
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// Bio order: Plex when it's longer than [`PLEX_BIO_ENOUGH`], else the first
/// of Wikipedia, Last.fm and TheAudioDB with more text than Plex has, else
/// whatever Plex has.
pub fn pick_bio(plex: Option<&str>, s: &Sources) -> Option<(String, BioSource)> {
    let plex = plex.map(str::trim).filter(|p| !p.is_empty());
    let plex_len = plex.map(chars).unwrap_or(0);
    if plex_len > PLEX_BIO_ENOUGH {
        return plex.map(|p| (p.to_owned(), BioSource::Plex));
    }
    let others = [
        (s.wiki.as_ref().and_then(|w| w.bio.as_deref()), BioSource::Wikipedia),
        (s.lastfm.as_ref().and_then(|l| l.bio.as_deref()), BioSource::Lastfm),
        (s.audiodb.as_ref().and_then(|a| a.bio.as_deref()), BioSource::Audiodb),
    ];
    for (text, src) in others {
        if let Some(t) = text.map(str::trim).filter(|t| chars(t) > plex_len) {
            return Some((t.to_owned(), src));
        }
    }
    plex.map(|p| (p.to_owned(), BioSource::Plex))
}

/// Most images in the About dialog's gallery.
pub const GALLERY_MAX: usize = 12;

/// Longest artist name (in words) looked for in a bio.
const BIO_NAME_WORDS: usize = 5;
/// Most names looked up per bio.
const BIO_CANDIDATES_MAX: usize = 1500;

/// A word of a bio: its byte range (edge punctuation and a trailing
/// possessive left out) and whether a sentence starts with it.
#[derive(Clone, Copy)]
struct Word {
    start: usize,
    end: usize,
    opens_sentence: bool,
}

fn bio_words(bio: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut opens = true;
    let mut i = 0;
    for raw in bio.split_inclusive(char::is_whitespace) {
        let at = i;
        i += raw.len();
        let word = raw.trim_end();
        if word.is_empty() {
            // A blank line starts a new paragraph.
            if raw.contains('\n') {
                opens = true;
            }
            continue;
        }
        let lead = word.len() - word.trim_start_matches(|c: char| !c.is_alphanumeric()).len();
        let mut core = word.trim_matches(|c: char| !c.is_alphanumeric());
        for poss in ["'s", "\u{2019}s"] {
            if let Some(c) = core.strip_suffix(poss) {
                core = c;
            }
        }
        if !core.is_empty() {
            out.push(Word { start: at + lead, end: at + lead + core.len(), opens_sentence: opens });
        }
        opens = word.ends_with(['.', '!', '?', ':']) || raw.ends_with('\n');
    }
    out
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
}

/// Spans (first word, end byte) of a bio that could be an artist's name, for
/// each first word the longest first: up to five words that start and end
/// with a capital (or digit), inside one line. A single word needs two
/// letters and must not open a sentence, where any word is capitalised.
fn bio_spans(bio: &str, words: &[Word]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..words.len() {
        let first = words[i];
        if !starts_upper(&bio[first.start..]) {
            continue;
        }
        for n in (1..=BIO_NAME_WORDS.min(words.len() - i)).rev() {
            let last = words[i + n - 1];
            if !starts_upper(&bio[last.start..last.end]) || bio[first.start..last.end].contains('\n') {
                continue;
            }
            if n == 1 && (first.opens_sentence || bio[first.start..first.end].chars().count() < 2) {
                continue;
            }
            out.push((i, last.end));
        }
    }
    out
}

/// Names in a bio worth looking up in the library.
pub fn bio_candidates(bio: &str) -> Vec<String> {
    let words = bio_words(bio);
    let mut seen = std::collections::HashSet::new();
    bio_spans(bio, &words)
        .into_iter()
        .map(|(i, end)| bio[words[i].start..end].split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|c| seen.insert(fold_name(c)))
        .take(BIO_CANDIDATES_MAX)
        .collect()
}

/// Library artists the bio names: the first mention of each, the longest
/// name where names overlap. `library` maps folded names to ids; `own` (the
/// artist the bio is about) is never linked.
pub fn bio_links(bio: &str, library: &HashMap<String, String>, own: &str) -> Vec<BioLink> {
    let words = bio_words(bio);
    let mut out: Vec<BioLink> = Vec::new();
    let mut next_free = 0;
    for (i, end) in bio_spans(bio, &words) {
        if i < next_free {
            continue;
        }
        let text = &bio[words[i].start..end];
        let Some(id) = library.get(&fold_name(text)) else { continue };
        if id == own {
            continue;
        }
        // Skip the shorter spans from this word and the words this name covers.
        next_free = words.iter().position(|w| w.start >= end).unwrap_or(words.len());
        if !out.iter().any(|l| l.artist_id == *id) {
            out.push(BioLink { text: text.to_owned(), artist_id: id.clone() });
        }
    }
    out
}

/// Gallery images: Plex's photo and background, then Fanart.tv's photos and
/// backgrounds, then TheAudioDB's photo and fan art, without repeats. Also
/// whether any came from Fanart.tv and from TheAudioDB.
fn gallery(plex: &PlexArtist, fanart: Option<&FanartData>, audiodb: Option<&AudioDbData>) -> (Vec<String>, bool, bool) {
    fn push(out: &mut Vec<String>, p: String) -> bool {
        let fits = !p.is_empty() && !out.contains(&p) && out.len() < GALLERY_MAX;
        if fits {
            out.push(p);
        }
        fits
    }
    let mut out: Vec<String> = Vec::new();
    for p in [plex.thumb.clone(), plex.art.clone()].into_iter().flatten() {
        push(&mut out, p);
    }
    let mut used_fanart = false;
    if let Some(f) = fanart {
        let thumbs: Vec<String> = if f.thumbs.is_empty() { f.thumb.iter().cloned().collect() } else { f.thumbs.clone() };
        let bgs: Vec<String> =
            if f.backgrounds.is_empty() { f.background.iter().cloned().collect() } else { f.backgrounds.clone() };
        for u in thumbs.into_iter().chain(bgs) {
            used_fanart |= push(&mut out, ext_art_path(&u));
        }
    }
    let mut used_audiodb = false;
    if let Some(a) = audiodb {
        for u in a.thumb.iter().chain(a.fanart.iter()) {
            used_audiodb |= push(&mut out, ext_art_path(u));
        }
    }
    (out, used_fanart, used_audiodb)
}

/// Website and profiles from Wikidata, then the Wikipedia article and the Last.fm page.
fn info_links(wiki: Option<&WikiData>, lastfm: Option<&LastfmData>) -> Vec<InfoLink> {
    let mut out: Vec<InfoLink> = wiki.map(|w| w.links.clone()).unwrap_or_default();
    if let Some(u) = wiki.and_then(|w| w.article_url.clone()) {
        out.push(InfoLink { kind: "wikipedia".into(), label: "Wikipedia".into(), url: u });
    }
    if let Some(u) = lastfm.and_then(|l| l.url.clone()) {
        out.push(InfoLink { kind: "lastfm".into(), label: "Last.fm".into(), url: u });
    }
    out
}

/// Merge Plex and the sources. `library` maps folded artist names to ids,
/// for linking members who are in the library.
pub fn merge(plex: &PlexArtist, s: &Sources, library: &HashMap<String, String>) -> ArtistInfo {
    let (bio, bio_source) = match pick_bio(plex.summary.as_deref(), s) {
        Some((b, src)) => (Some(b), Some(src)),
        None => (None, None),
    };
    let wiki = s.wiki.as_ref();
    let mb = s.musicbrainz.as_ref();
    let link = |name: &str| library.get(&fold_name(name)).filter(|id| **id != plex.id).cloned();
    // MusicBrainz members carry their years, so they go first.
    let mb_members = mb.is_some_and(|m| !m.members.is_empty());
    let members: Vec<Member> = match (mb, wiki) {
        (Some(m), _) if mb_members => {
            m.members.iter().map(|x| Member { name: x.name.clone(), artist_id: link(&x.name), years: x.years.clone() }).collect()
        }
        (_, Some(w)) => w.members.iter().map(|name| Member { name: name.clone(), artist_id: link(name), years: None }).collect(),
        _ => Vec::new(),
    };
    let related: Vec<Related> = mb
        .map(|m| {
            m.related
                .iter()
                .map(|r| Related { name: r.name.clone(), artist_id: link(&r.name), relation: r.relation.clone() })
                .collect()
        })
        .unwrap_or_default();
    let origin = wiki.and_then(|w| w.origin.clone());
    let wiki_years = wiki.and_then(|w| w.years_active.clone());
    let mb_years = if wiki_years.is_none() { mb.and_then(|m| m.life_span.clone()) } else { None };
    let years_active = wiki_years.or(mb_years.clone());
    let lastfm = s.lastfm.as_ref();
    let tags = lastfm.map(|l| l.tags.clone()).unwrap_or_default();
    // Last.fm's numbers, else ListenBrainz's.
    let lb = s.lb_stats.as_ref().filter(|_| lastfm.is_none_or(|l| l.listeners.is_none()));
    let (listeners, scrobbles, listeners_from) = match (lastfm.and_then(|l| l.listeners), lb) {
        (Some(n), _) => (Some(n), lastfm.and_then(|l| l.playcount), Some("lastfm")),
        (None, Some(st)) if st.listeners.is_some() => (st.listeners, st.listens, Some("listenbrainz")),
        _ => (None, lastfm.and_then(|l| l.playcount), lastfm.and_then(|l| l.playcount).map(|_| "lastfm")),
    };

    // Plex art always wins.
    let has_thumb = plex.thumb.as_deref().is_some_and(|t| !t.is_empty());
    let has_art = plex.art.as_deref().is_some_and(|t| !t.is_empty());
    let fanart = s.fanart.as_ref();
    let audiodb = s.audiodb.as_ref();
    let fanart_photo = if has_thumb { None } else { fanart.and_then(|f| f.thumb.clone()) };
    let audiodb_photo = if has_thumb || fanart_photo.is_some() { None } else { audiodb.and_then(|a| a.thumb.clone()) };
    let banner = if has_art { None } else { fanart.and_then(|f| f.background.clone()) };
    let logo = fanart.and_then(|f| f.logo.clone());
    let (gallery, gallery_fanart, gallery_audiodb) = gallery(plex, fanart, audiodb);
    let bio_links = bio.as_deref().map(|b| bio_links(b, library, &plex.id)).unwrap_or_default();

    let mut credits = Vec::new();
    if bio_source == Some(BioSource::Wikipedia)
        && let Some(w) = wiki
    {
        credits.push(Credit {
            source: "wikipedia".into(),
            text: "From Wikipedia".into(),
            url: w.article_url.clone(),
            licence: Some(WIKIPEDIA_LICENCE.into()),
            licence_url: Some(WIKIPEDIA_LICENCE_URL.into()),
        });
    }
    if let Some(w) = wiki
        && (origin.is_some() || wiki.is_some_and(|w| w.years_active.is_some()) || (!members.is_empty() && !mb_members))
    {
        credits.push(Credit {
            source: "wikidata".into(),
            text: "Facts from Wikidata".into(),
            url: Some(format!("https://www.wikidata.org/wiki/{}", w.qid)),
            licence: Some("CC0".into()),
            licence_url: Some("https://creativecommons.org/publicdomain/zero/1.0/".into()),
        });
    }
    if let Some(l) = lastfm
        && (bio_source == Some(BioSource::Lastfm) || listeners.is_some() || !tags.is_empty())
    {
        let bio = bio_source == Some(BioSource::Lastfm);
        credits.push(Credit {
            source: "lastfm".into(),
            text: "Data from Last.fm".into(),
            url: l.url.clone().or_else(|| Some("https://www.last.fm".into())),
            licence: bio.then(|| LASTFM_LICENCE.into()),
            licence_url: bio.then(|| LASTFM_LICENCE_URL.into()),
        });
    }
    if let Some(m) = mb
        && (mb_members || !related.is_empty() || mb_years.is_some())
    {
        credits.push(Credit {
            source: "musicbrainz".into(),
            text: "Members and relationships from MusicBrainz".into(),
            url: Some(format!("https://musicbrainz.org/artist/{}", m.mbid)),
            licence: Some("CC0".into()),
            licence_url: Some("https://creativecommons.org/publicdomain/zero/1.0/".into()),
        });
    }
    if listeners_from == Some("listenbrainz")
        && let Some(st) = lb
    {
        credits.push(Credit {
            source: "listenbrainz".into(),
            text: "Listener numbers from ListenBrainz".into(),
            url: Some(format!("https://listenbrainz.org/artist/{}/", st.mbid)),
            licence: Some("CC0".into()),
            licence_url: Some("https://creativecommons.org/publicdomain/zero/1.0/".into()),
        });
    }
    if let Some(f) = fanart
        && (fanart_photo.is_some() || banner.is_some() || logo.is_some() || gallery_fanart)
    {
        credits.push(Credit {
            source: "fanart".into(),
            text: "Images from Fanart.tv".into(),
            url: Some(format!("https://fanart.tv/artist/{}", f.mbid)),
            licence: None,
            licence_url: None,
        });
    }
    if let Some(a) = audiodb
        && (bio_source == Some(BioSource::Audiodb) || audiodb_photo.is_some() || gallery_audiodb)
    {
        credits.push(Credit {
            source: "audiodb".into(),
            text: "From TheAudioDB".into(),
            url: a.url.clone().or_else(|| Some("https://www.theaudiodb.com".into())),
            licence: None,
            licence_url: None,
        });
    }

    ArtistInfo {
        id: plex.id.clone(),
        bio,
        bio_source,
        origin,
        years_active,
        members,
        tags,
        listeners,
        scrobbles,
        listeners_from: listeners_from.map(str::to_owned),
        related,
        photo: fanart_photo.or(audiodb_photo).map(|u| ext_art_path(&u)),
        banner: banner.map(|u| ext_art_path(&u)),
        logo: logo.map(|u| ext_art_path(&u)),
        credits,
        gallery,
        links: info_links(wiki, lastfm),
        bio_links,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiki(bio: &str) -> WikiData {
        WikiData {
            qid: "Q1".into(),
            bio: Some(bio.into()),
            article_url: Some("https://en.wikipedia.org/wiki/Band".into()),
            origin: Some("Abingdon".into()),
            years_active: Some("1985–present".into()),
            members: vec!["Singer".into(), "Drummer".into()],
            mbid: None,
            links: Vec::new(),
        }
    }

    fn lastfm(bio: &str) -> LastfmData {
        LastfmData {
            url: Some("https://www.last.fm/music/Band".into()),
            listeners: Some(5_000),
            playcount: Some(90_000),
            tags: vec!["rock".into()],
            bio: Some(bio.into()),
            mbid: None,
        }
    }

    fn audiodb(bio: &str) -> AudioDbData {
        AudioDbData {
            url: Some("https://www.theaudiodb.com/artist/1".into()),
            bio: Some(bio.into()),
            thumb: Some("https://r2.theaudiodb.com/t.jpg".into()),
            fanart: Vec::new(),
        }
    }

    fn plex(summary: Option<&str>) -> PlexArtist {
        PlexArtist {
            id: "1".into(),
            title: "Band".into(),
            summary: summary.map(str::to_owned),
            thumb: Some("/library/metadata/1/thumb/1".into()),
            art: None,
        }
    }

    #[test]
    fn bio_order_plex_long_then_wikipedia_lastfm_audiodb() {
        let long_plex = "p".repeat(301);
        let all = Sources {
            wiki: Some(wiki(&"w".repeat(900))),
            lastfm: Some(lastfm(&"l".repeat(900))),
            audiodb: Some(audiodb(&"a".repeat(900))),
            fanart: None,
            ..Default::default()
        };
        assert_eq!(pick_bio(Some(&long_plex), &all).unwrap().1, BioSource::Plex);
        assert_eq!(pick_bio(Some("short"), &all).unwrap().1, BioSource::Wikipedia);
        assert_eq!(pick_bio(None, &all).unwrap().1, BioSource::Wikipedia);
        let no_wiki = Sources { wiki: None, ..all.clone() };
        assert_eq!(pick_bio(None, &no_wiki).unwrap().1, BioSource::Lastfm);
        let only_db = Sources { audiodb: all.audiodb.clone(), ..Default::default() };
        assert_eq!(pick_bio(None, &only_db).unwrap().1, BioSource::Audiodb);
        assert_eq!(pick_bio(None, &Sources::default()), None);
    }

    #[test]
    fn a_short_plex_summary_stays_unless_another_source_has_more() {
        let s = Sources { wiki: Some(wiki("tiny")), lastfm: Some(lastfm("A little longer than Plex.")), ..Default::default() };
        // Wikipedia has less than Plex; Last.fm has more.
        assert_eq!(pick_bio(Some("Plex says hi."), &s).unwrap().1, BioSource::Lastfm);
        let s = Sources { wiki: Some(wiki("tiny")), ..Default::default() };
        assert_eq!(pick_bio(Some("Plex says hi."), &s).unwrap(), ("Plex says hi.".into(), BioSource::Plex));
    }

    #[test]
    fn credits_follow_what_is_shown() {
        let s = Sources { wiki: Some(wiki(&"w".repeat(400))), lastfm: Some(lastfm("l")), ..Default::default() };
        let info = merge(&plex(None), &s, &HashMap::new());
        let sources: Vec<&str> = info.credits.iter().map(|c| c.source.as_str()).collect();
        assert_eq!(sources, ["wikipedia", "wikidata", "lastfm"]);
        let wp = &info.credits[0];
        assert_eq!(wp.text, "From Wikipedia");
        assert_eq!(wp.licence.as_deref(), Some("CC BY-SA 4.0"));
        assert_eq!(wp.url.as_deref(), Some("https://en.wikipedia.org/wiki/Band"));
        assert_eq!(info.credits[2].text, "Data from Last.fm");
        assert_eq!(info.listeners, Some(5_000));

        // Plex bio long enough: no Wikipedia text credit, but its facts still credit Wikidata.
        let info = merge(&plex(Some(&"p".repeat(400))), &s, &HashMap::new());
        assert!(!info.credits.iter().any(|c| c.source == "wikipedia"));
        assert!(info.credits.iter().any(|c| c.source == "wikidata"));
        assert_eq!(info.bio_source, Some(BioSource::Plex));

        // Nothing from outside: no credits.
        let info = merge(&plex(Some("x")), &Sources::default(), &HashMap::new());
        assert!(info.credits.is_empty());
    }

    #[test]
    fn plex_art_wins_and_fanart_fills_gaps() {
        let f = FanartData {
            mbid: "m".into(),
            background: Some("https://assets.fanart.tv/bg.jpg".into()),
            thumb: Some("https://assets.fanart.tv/th.jpg".into()),
            logo: None,
            ..Default::default()
        };
        let s = Sources { fanart: Some(f), audiodb: Some(audiodb("")), ..Default::default() };
        let with_art = PlexArtist { art: Some("/a".into()), ..plex(None) };
        let info = merge(&with_art, &s, &HashMap::new());
        assert_eq!((info.photo, info.banner), (None, None));
        // Their images still join the gallery, after Plex's, so they're credited.
        assert_eq!(
            info.gallery,
            [
                "/library/metadata/1/thumb/1",
                "/a",
                "/ext/https://assets.fanart.tv/th.jpg",
                "/ext/https://assets.fanart.tv/bg.jpg",
                "/ext/https://r2.theaudiodb.com/t.jpg"
            ]
        );
        assert_eq!(info.credits.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["fanart", "audiodb"]);

        let bare = PlexArtist { thumb: None, art: None, ..plex(None) };
        let info = merge(&bare, &s, &HashMap::new());
        assert_eq!(info.photo.as_deref(), Some("/ext/https://assets.fanart.tv/th.jpg"));
        assert_eq!(info.banner.as_deref(), Some("/ext/https://assets.fanart.tv/bg.jpg"));
        assert_eq!(info.credits.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["fanart", "audiodb"]);

        // No Fanart.tv photo: TheAudioDB's.
        let s = Sources { audiodb: Some(audiodb("")), ..Default::default() };
        let info = merge(&bare, &s, &HashMap::new());
        assert_eq!(info.photo.as_deref(), Some("/ext/https://r2.theaudiodb.com/t.jpg"));
        assert_eq!(info.credits[0].source, "audiodb");
    }

    #[test]
    fn gallery_takes_every_source_image_once_up_to_the_cap() {
        let many: Vec<String> = (0..20).map(|i| format!("https://assets.fanart.tv/{i}.jpg")).collect();
        let f = FanartData { mbid: "m".into(), thumbs: many.clone(), backgrounds: many[..3].to_vec(), ..Default::default() };
        let a = AudioDbData { fanart: vec!["https://r2.theaudiodb.com/f.jpg".into()], ..Default::default() };
        let s = Sources { fanart: Some(f), audiodb: Some(a), ..Default::default() };
        let info = merge(&plex(None), &s, &HashMap::new());
        assert_eq!(info.gallery.len(), GALLERY_MAX);
        assert_eq!(info.gallery[0], "/library/metadata/1/thumb/1");
        assert_eq!(info.gallery[1], "/ext/https://assets.fanart.tv/0.jpg");
        // Full before TheAudioDB's turn: not credited.
        assert!(!info.credits.iter().any(|c| c.source == "audiodb"));
        let info = merge(&PlexArtist { thumb: None, ..plex(None) }, &Sources::default(), &HashMap::new());
        assert!(info.gallery.is_empty());
    }

    #[test]
    fn links_list_the_profiles_then_wikipedia_and_lastfm() {
        let mut w = wiki("w");
        w.links = vec![InfoLink { kind: "website".into(), label: "Official website".into(), url: "https://band.example/".into() }];
        let s = Sources { wiki: Some(w), lastfm: Some(lastfm("l")), ..Default::default() };
        let info = merge(&plex(None), &s, &HashMap::new());
        let kinds: Vec<&str> = info.links.iter().map(|l| l.kind.as_str()).collect();
        assert_eq!(kinds, ["website", "wikipedia", "lastfm"]);
    }

    #[test]
    fn bio_names_artists_in_the_library_once_each() {
        let bio = "Band formed with Mara Quill. They toured with The Night Owls and later with Mara Quill again.\n\nOwls fans know Band's sound. The Night Owls' drummer joined.";
        let lib = HashMap::from([
            ("mara quill".to_string(), "5".to_string()),
            ("the night owls".to_string(), "6".to_string()),
            ("owls".to_string(), "7".to_string()),
            ("band".to_string(), "1".to_string()),
        ]);
        let links = bio_links(bio, &lib, "1");
        let got: Vec<(&str, &str)> = links.iter().map(|l| (l.text.as_str(), l.artist_id.as_str())).collect();
        // The longest name wins inside "The Night Owls"; "Owls" opening a sentence isn't linked;
        // the artist itself never is.
        assert_eq!(got, [("Mara Quill", "5"), ("The Night Owls", "6")]);
        let c = bio_candidates(bio);
        assert!(c.contains(&"The Night Owls".to_string()));
        assert!(c.contains(&"Mara Quill".to_string()));
    }

    #[test]
    fn a_single_word_name_mid_sentence_is_linked() {
        let lib = HashMap::from([("seahorse".to_string(), "9".to_string())]);
        let links = bio_links("Their first tour was with (Seahorse), in vans.", &lib, "1");
        assert_eq!(links, [BioLink { text: "Seahorse".into(), artist_id: "9".into() }]);
        assert!(bio_links("Seahorse opened.", &lib, "1").is_empty());
    }

    #[test]
    fn members_in_the_library_are_linked() {
        let s = Sources { wiki: Some(wiki("w")), ..Default::default() };
        let lib = HashMap::from([("singer".to_string(), "77".to_string())]);
        let info = merge(&plex(None), &s, &lib);
        assert_eq!(info.members[0], Member { name: "Singer".into(), artist_id: Some("77".into()), years: None });
        assert_eq!(info.members[1].artist_id, None);
        assert_eq!(info.origin.as_deref(), Some("Abingdon"));
    }
}
