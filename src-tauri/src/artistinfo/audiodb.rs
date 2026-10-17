//! TheAudioDB (the user's own key; its terms only let free keys power
//! development projects). Uses the v2 API, where the key travels in the
//! `X-API-KEY` header. Photo and bio, last in line.

use serde_json::Value;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{AudioDbData, Fetch, FetchError, clean_text, fold_name, is_mbid};

pub const API: &str = "https://www.theaudiodb.com/api/v2/json/";
/// The free tier (v1, its public test key), used only for photos of artists
/// outside the library when the user has no key of their own.
pub const FREE_API: &str = "https://www.theaudiodb.com/api/v1/json/123/";

/// An artist by MusicBrainz id on the free tier.
pub async fn free_lookup(http: &dyn InfoHttp, limiter: &Limiter, mbid: &str) -> Fetch<AudioDbData> {
    if !is_mbid(mbid) {
        return Ok(None);
    }
    let url = format!("{FREE_API}artist-mb.php?i={}", mbid.to_ascii_lowercase());
    let resp = limiter.get(http, &url, &[]).await?;
    match resp.status {
        404 => Ok(None),
        s if (200..300).contains(&s) => {
            // An unknown artist comes back as `{"artists": null}`.
            let v: Value = resp.json()?;
            Ok(rows(&v).first().and_then(|r| to_data(r)))
        }
        s => Err(FetchError::Unavailable(format!("TheAudioDB answered {s}"))),
    }
}

pub struct AudioDb<'a> {
    pub http: &'a dyn InfoHttp,
    pub limiter: &'a Limiter,
    pub key: &'a str,
}

/// Artist rows from a reply. The list's key differs between endpoints
/// ("artists", "lookup", "search"), so take the first list of objects.
fn rows(v: &Value) -> Vec<&Value> {
    v.as_object()
        .and_then(|o| o.values().find_map(|x| x.as_array().filter(|a| a.iter().all(Value::is_object))))
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

pub fn to_data(row: &Value) -> Option<AudioDbData> {
    let d = AudioDbData {
        url: text(row, "idArtist").map(|id| format!("https://www.theaudiodb.com/artist/{id}")),
        bio: text(row, "strBiographyEN").map(|b| clean_text(&b)).filter(|b| !b.is_empty()),
        thumb: text(row, "strArtistThumb").filter(|u| u.starts_with("https://")),
        fanart: ["strArtistFanart", "strArtistFanart2", "strArtistFanart3", "strArtistFanart4", "strArtistWideThumb"]
            .iter()
            .filter_map(|k| text(row, k))
            .filter(|u| u.starts_with("https://"))
            .collect(),
    };
    (d.bio.is_some() || d.thumb.is_some() || !d.fanart.is_empty()).then_some(d)
}

impl AudioDb<'_> {
    async fn get(&self, path: &str) -> Result<Option<Value>, FetchError> {
        let url = format!("{API}{path}");
        let resp = self.limiter.get(self.http, &url, &[("X-API-KEY", self.key.to_owned())]).await?;
        match resp.status {
            401 | 403 => Err(FetchError::KeyRejected),
            404 => Ok(None),
            s if (200..300).contains(&s) => Ok(Some(resp.json()?)),
            s => Err(FetchError::Unavailable(format!("TheAudioDB answered {s}"))),
        }
    }

    /// By MusicBrainz id, else a name search that must find exactly one
    /// artist with that name.
    pub async fn lookup(&self, mbid: Option<&str>, name: &str) -> Fetch<AudioDbData> {
        if let Some(m) = mbid.filter(|m| is_mbid(m))
            && let Some(v) = self.get(&format!("lookup/artist_mb/{m}")).await?
            && let Some(row) = rows(&v).first()
        {
            return Ok(to_data(row));
        }
        if name.trim().is_empty() {
            return Ok(None);
        }
        let q = percent_encoding::utf8_percent_encode(name.trim(), percent_encoding::NON_ALPHANUMERIC).to_string();
        let Some(v) = self.get(&format!("search/artist/{q}")).await? else { return Ok(None) };
        let want = fold_name(name);
        let hits: Vec<&Value> = rows(&v)
            .into_iter()
            .filter(|r| text(r, "strArtist").is_some_and(|n| fold_name(&n) == want))
            .collect();
        Ok(match hits.as_slice() {
            [one] => to_data(one),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rows_under_any_list_key() {
        let v = serde_json::json!({"artists": [{"idArtist": "111", "strArtist": "Band", "strBiographyEN": "Bio.", "strArtistThumb": "https://r2.theaudiodb.com/images/media/artist/thumb/x.jpg",
            "strArtistFanart": "https://r2.theaudiodb.com/images/media/artist/fanart/f1.jpg", "strArtistFanart2": "http://r2.theaudiodb.com/f2.jpg", "strArtistFanart3": ""}]});
        let d = to_data(rows(&v)[0]).unwrap();
        assert_eq!(d.fanart, ["https://r2.theaudiodb.com/images/media/artist/fanart/f1.jpg"]);
        assert_eq!(d.url.as_deref(), Some("https://www.theaudiodb.com/artist/111"));
        assert_eq!(d.bio.as_deref(), Some("Bio."));
        let v = serde_json::json!({"search": null, "lookup": [{"strArtist": "B"}]});
        assert_eq!(rows(&v).len(), 1);
        assert_eq!(to_data(rows(&v)[0]), None);
    }
}
