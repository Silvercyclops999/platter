//! Lidarr's own artist and album search (its metadata service), for
//! catalogue results in search and pictures of artists outside the library.
//! Reads only, with no one-a-second limit like MusicBrainz's.

use super::Lidarr;
use super::api::{Album, Artist, LookupImage};
use crate::artistinfo::art::ext_url;
use crate::artistinfo::{ext_art_path, is_mbid};
use crate::error::AppResult;

impl Lidarr {
    /// An address and a key are set, so lookups can be asked.
    pub fn lookups_ready(&self) -> bool {
        self.status().usable()
    }

    /// Artists matching search words.
    pub async fn lookup_artists(&self, term: &str) -> AppResult<Vec<Artist>> {
        let (client, _) = self.connection().await?;
        client.artist_search(term).await
    }

    /// Albums matching search words.
    pub async fn lookup_albums(&self, term: &str) -> AppResult<Vec<Album>> {
        let (client, _) = self.connection().await?;
        client.album_search(term).await
    }

    /// One artist by MusicBrainz id.
    pub async fn lookup_artist(&self, mbid: &str) -> AppResult<Option<Artist>> {
        if !is_mbid(mbid) {
            return Ok(None);
        }
        let (client, _) = self.connection().await?;
        let list = client.artist_search(&format!("lidarr:{}", mbid.to_ascii_lowercase())).await?;
        Ok(list.into_iter().find(|a| a.foreign_artist_id.eq_ignore_ascii_case(mbid)))
    }
}

/// A lookup picture as an art path, when it's on an image host Platter
/// fetches from (plain http is asked over https).
pub fn image_path(img: &LookupImage) -> Option<String> {
    let raw = img.remote_url.as_deref()?.trim();
    let https = match raw.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => raw.to_string(),
    };
    let path = ext_art_path(&https);
    ext_url(&path).is_some().then_some(path)
}

/// The first usable picture of these kinds, in this order.
pub fn pick_image(images: &[LookupImage], kinds: &[&str]) -> Option<String> {
    kinds
        .iter()
        .find_map(|k| images.iter().filter(|i| i.cover_type.eq_ignore_ascii_case(k)).find_map(image_path))
}

/// An artist's photo from a lookup: the portrait, else the wide picture.
pub fn artist_photo(a: &Artist) -> Option<String> {
    pick_image(&a.images, &["poster", "fanart", "thumb"])
}

/// An album's front cover from a lookup.
pub fn album_cover(a: &Album) -> Option<String> {
    pick_image(&a.images, &["cover", "poster"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(kind: &str, url: &str) -> LookupImage {
        LookupImage { cover_type: kind.into(), remote_url: Some(url.into()) }
    }

    #[test]
    fn pictures_only_from_allowed_hosts_over_https() {
        let a = Artist {
            images: vec![
                img("banner", "https://assets.fanart.tv/b.jpg"),
                img("poster", "https://evil.example/p.jpg"),
                img("poster", "http://assets.fanart.tv/p.jpg"),
            ],
            ..Default::default()
        };
        assert_eq!(artist_photo(&a).as_deref(), Some("/ext/https://assets.fanart.tv/p.jpg"), "other hosts are skipped; http asked over https");
        let none = Artist { images: vec![img("logo", "https://assets.fanart.tv/l.png")], ..Default::default() };
        assert_eq!(artist_photo(&none), None, "a logo isn't a photo");
        let al = Album { images: vec![img("cover", "https://coverartarchive.org/release/x/front.jpg")], ..Default::default() };
        assert_eq!(album_cover(&al).as_deref(), Some("/ext/https://coverartarchive.org/release/x/front.jpg"));
    }
}
