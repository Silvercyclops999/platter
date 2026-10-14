//! What the artist page shows: Plex first, the outside sources filling gaps.
//! Pure, so the order rules are easy to test.

use std::collections::HashMap;

use super::{
    ArtistInfo, AudioDbData, BioSource, Credit, FanartData, LastfmData, Member, WikiData, ext_art_path, fold_name,
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

/// Merge Plex and the sources. `library` maps folded artist names to ids,
/// for linking members who are in the library.
pub fn merge(plex: &PlexArtist, s: &Sources, library: &HashMap<String, String>) -> ArtistInfo {
    let (bio, bio_source) = match pick_bio(plex.summary.as_deref(), s) {
        Some((b, src)) => (Some(b), Some(src)),
        None => (None, None),
    };
    let wiki = s.wiki.as_ref();
    let members: Vec<Member> = wiki
        .map(|w| {
            w.members
                .iter()
                .map(|name| Member {
                    name: name.clone(),
                    artist_id: library.get(&fold_name(name)).filter(|id| **id != plex.id).cloned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let origin = wiki.and_then(|w| w.origin.clone());
    let years_active = wiki.and_then(|w| w.years_active.clone());
    let lastfm = s.lastfm.as_ref();
    let tags = lastfm.map(|l| l.tags.clone()).unwrap_or_default();
    let listeners = lastfm.and_then(|l| l.listeners);
    let scrobbles = lastfm.and_then(|l| l.playcount);

    // Plex art always wins.
    let has_thumb = plex.thumb.as_deref().is_some_and(|t| !t.is_empty());
    let has_art = plex.art.as_deref().is_some_and(|t| !t.is_empty());
    let fanart = s.fanart.as_ref();
    let audiodb = s.audiodb.as_ref();
    let fanart_photo = if has_thumb { None } else { fanart.and_then(|f| f.thumb.clone()) };
    let audiodb_photo = if has_thumb || fanart_photo.is_some() { None } else { audiodb.and_then(|a| a.thumb.clone()) };
    let banner = if has_art { None } else { fanart.and_then(|f| f.background.clone()) };
    let logo = fanart.and_then(|f| f.logo.clone());

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
        && (origin.is_some() || years_active.is_some() || !members.is_empty())
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
    if let Some(f) = fanart
        && (fanart_photo.is_some() || banner.is_some() || logo.is_some())
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
        && (bio_source == Some(BioSource::Audiodb) || audiodb_photo.is_some())
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
        photo: fanart_photo.or(audiodb_photo).map(|u| ext_art_path(&u)),
        banner: banner.map(|u| ext_art_path(&u)),
        logo: logo.map(|u| ext_art_path(&u)),
        credits,
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
        }
    }

    fn lastfm(bio: &str) -> LastfmData {
        LastfmData {
            url: Some("https://www.last.fm/music/Band".into()),
            listeners: Some(5_000),
            playcount: Some(90_000),
            tags: vec!["rock".into()],
            bio: Some(bio.into()),
        }
    }

    fn audiodb(bio: &str) -> AudioDbData {
        AudioDbData { url: Some("https://www.theaudiodb.com/artist/1".into()), bio: Some(bio.into()), thumb: Some("https://r2.theaudiodb.com/t.jpg".into()) }
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
        };
        let s = Sources { fanart: Some(f), audiodb: Some(audiodb("")), ..Default::default() };
        let with_art = PlexArtist { art: Some("/a".into()), ..plex(None) };
        let info = merge(&with_art, &s, &HashMap::new());
        assert_eq!((info.photo, info.banner), (None, None));
        assert!(info.credits.is_empty());

        let bare = PlexArtist { thumb: None, art: None, ..plex(None) };
        let info = merge(&bare, &s, &HashMap::new());
        assert_eq!(info.photo.as_deref(), Some("/ext/https://assets.fanart.tv/th.jpg"));
        assert_eq!(info.banner.as_deref(), Some("/ext/https://assets.fanart.tv/bg.jpg"));
        assert_eq!(info.credits.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["fanart"]);

        // No Fanart.tv photo: TheAudioDB's.
        let s = Sources { audiodb: Some(audiodb("")), ..Default::default() };
        let info = merge(&bare, &s, &HashMap::new());
        assert_eq!(info.photo.as_deref(), Some("/ext/https://r2.theaudiodb.com/t.jpg"));
        assert_eq!(info.credits[0].source, "audiodb");
    }

    #[test]
    fn members_in_the_library_are_linked() {
        let s = Sources { wiki: Some(wiki("w")), ..Default::default() };
        let lib = HashMap::from([("singer".to_string(), "77".to_string())]);
        let info = merge(&plex(None), &s, &lib);
        assert_eq!(info.members[0], Member { name: "Singer".into(), artist_id: Some("77".into()) });
        assert_eq!(info.members[1].artist_id, None);
        assert_eq!(info.origin.as_deref(), Some("Abingdon"));
    }
}
