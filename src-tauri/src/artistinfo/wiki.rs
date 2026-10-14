//! Wikidata and Wikipedia (no key).
//!
//! Matching: the MusicBrainz id finds the Wikidata item by its P434 statement.
//! Without one (or when no item has it), search Wikidata by name and accept
//! only a single item whose label is the name and that is a musician or a
//! band. Anything else shows nothing; no guessing. The item gives origin,
//! years active and members, and its Wikipedia sitelink gives the article,
//! whose lead section becomes the bio.

use std::collections::HashMap;

use serde_json::Value;
use url::Url;

use super::http::InfoHttp;
use super::limit::Limiter;
use super::{Fetch, FetchError, WikiData, clean_text, fold_name, is_mbid};

pub const WIKIDATA_API: &str = "https://www.wikidata.org/w/api.php";

/// Instance-of (P31) values that mean a band or other music group.
const GROUP_TYPES: &[&str] = &[
    "Q215380",   // musical group
    "Q5741069",  // rock band
    "Q2088357",  // musical ensemble
    "Q9212979",  // musical duo
    "Q281643",   // musical trio
    "Q641066",   // girl group
    "Q216337",   // boy band
    "Q56816954", // heavy metal band
    "Q7628270",  // studio band
];

/// Occupation (P106) values that mean a person makes music.
const MUSIC_OCCUPATIONS: &[&str] = &[
    "Q639669",  // musician
    "Q177220",  // singer
    "Q36834",   // composer
    "Q753110",  // songwriter
    "Q488205",  // singer-songwriter
    "Q2252262", // rapper
    "Q130857",  // disc jockey
    "Q855091",  // guitarist
    "Q183945",  // record producer
    "Q386854",  // drummer
    "Q486748",  // pianist
    "Q584301",  // bassist
];

const MAX_MEMBERS: usize = 24;

pub struct Wiki<'a> {
    pub http: &'a dyn InfoHttp,
    pub limiter: &'a Limiter,
    /// Wikipedia language edition ("en").
    pub lang: &'a str,
}

fn api(base: &str, params: &[(&str, &str)]) -> String {
    let mut u = Url::parse(base).expect("static url");
    u.query_pairs_mut()
        .extend_pairs(params)
        .append_pair("format", "json")
        .append_pair("formatversion", "2");
    u.into()
}

/// Entity ids (Q…) of a claim's values, skipping deprecated statements.
fn claim_ids(entity: &Value, prop: &str) -> Vec<String> {
    claim_values(entity, prop)
        .filter_map(|v| v.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn claim_values<'a>(entity: &'a Value, prop: &str) -> impl Iterator<Item = &'a Value> {
    let claims = entity.pointer(&format!("/claims/{prop}")).and_then(Value::as_array);
    let mut list: Vec<&Value> = claims.map(|a| a.iter().collect()).unwrap_or_default();
    // Preferred statements first.
    list.sort_by_key(|c| match c.get("rank").and_then(Value::as_str) {
        Some("preferred") => 0,
        _ => 1,
    });
    list.into_iter()
        .filter(|c| c.get("rank").and_then(Value::as_str) != Some("deprecated"))
        .filter_map(|c| c.pointer("/mainsnak/datavalue/value"))
}

fn claim_year(entity: &Value, prop: &str) -> Option<i32> {
    let t = claim_values(entity, prop).next()?.get("time")?.as_str()?;
    // "+1985-00-00T00:00:00Z"
    let t = t.strip_prefix('+').unwrap_or(t);
    let year: i32 = t.split('-').next()?.parse().ok()?;
    (year > 0).then_some(year)
}

fn claim_string(entity: &Value, prop: &str) -> Option<String> {
    claim_values(entity, prop).next()?.as_str().map(str::to_owned)
}

/// A band, or a person with a music occupation, or anything with a
/// MusicBrainz artist id.
pub fn is_music_artist(entity: &Value) -> bool {
    if claim_string(entity, "P434").is_some() {
        return true;
    }
    if claim_ids(entity, "P31").iter().any(|q| GROUP_TYPES.contains(&q.as_str())) {
        return true;
    }
    claim_ids(entity, "P31").iter().any(|q| q == "Q5")
        && claim_ids(entity, "P106").iter().any(|q| MUSIC_OCCUPATIONS.contains(&q.as_str()))
}

fn is_group(entity: &Value) -> bool {
    claim_ids(entity, "P31").iter().any(|q| GROUP_TYPES.contains(&q.as_str()))
}

/// "1985–present", "1964–2009", or None.
pub fn years_active(start: Option<i32>, end: Option<i32>) -> Option<String> {
    match (start, end) {
        (Some(s), Some(e)) if e > s => Some(format!("{s}–{e}")),
        (Some(s), Some(_)) => Some(s.to_string()),
        (Some(s), None) => Some(format!("{s}–present")),
        (None, Some(e)) => Some(format!("until {e}")),
        (None, None) => None,
    }
}

impl Wiki<'_> {
    async fn get_json(&self, url: &str) -> Result<Value, FetchError> {
        let resp = self.limiter.get(self.http, url, &[]).await?;
        if !resp.ok() {
            return Err(FetchError::Unavailable(format!("wiki answered {}", resp.status)));
        }
        Ok(resp.json()?)
    }

    async fn entities(&self, ids: &[String], props: &str) -> Result<Value, FetchError> {
        let joined = ids.join("|");
        let url = api(
            WIKIDATA_API,
            &[
                ("action", "wbgetentities"),
                ("ids", &joined),
                ("props", props),
                ("languages", &format!("{}|mul", self.lang)),
                ("languagefallback", "1"),
                ("sitefilter", &format!("{}wiki", self.lang)),
            ],
        );
        Ok(self.get_json(&url).await?.get("entities").cloned().unwrap_or(Value::Null))
    }

    /// The Wikidata item with this MusicBrainz artist id, when exactly one has it.
    async fn qid_by_mbid(&self, mbid: &str) -> Result<Option<String>, FetchError> {
        let q = format!("haswbstatement:P434={mbid}");
        let url = api(WIKIDATA_API, &[("action", "query"), ("list", "search"), ("srsearch", &q), ("srlimit", "2")]);
        let v = self.get_json(&url).await?;
        let hits: Vec<&str> = v
            .pointer("/query/search")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|h| h.get("title").and_then(Value::as_str)).collect())
            .unwrap_or_default();
        Ok(match hits.as_slice() {
            [one] => Some((*one).to_owned()),
            _ => None,
        })
    }

    /// Name search: the single item labelled with the name that is a music
    /// artist, or None when there are none or several.
    async fn qid_by_name(&self, name: &str) -> Result<Option<(String, Value)>, FetchError> {
        let url = api(
            WIKIDATA_API,
            &[
                ("action", "wbsearchentities"),
                ("search", name),
                ("language", self.lang),
                ("uselang", self.lang),
                ("type", "item"),
                ("limit", "10"),
            ],
        );
        let v = self.get_json(&url).await?;
        let want = fold_name(name);
        let ids: Vec<String> = v
            .get("search")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|h| {
                        let label = h.get("label").and_then(Value::as_str).unwrap_or("");
                        let matched = h.pointer("/match/text").and_then(Value::as_str).unwrap_or("");
                        fold_name(label) == want || fold_name(matched) == want
                    })
                    .filter_map(|h| h.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok(None);
        }
        let ents = self.entities(&ids, "claims|sitelinks").await?;
        let mut found: Vec<(String, Value)> = ids
            .iter()
            .filter_map(|id| ents.get(id).map(|e| (id.clone(), e.clone())))
            .filter(|(_, e)| is_music_artist(e))
            .collect();
        Ok(if found.len() == 1 { found.pop() } else { None })
    }

    /// Lead section of the article as plain text.
    async fn extract(&self, title: &str) -> Result<Option<String>, FetchError> {
        let base = format!("https://{}.wikipedia.org/w/api.php", self.lang);
        let url = api(
            &base,
            &[
                ("action", "query"),
                ("prop", "extracts"),
                ("exintro", "1"),
                ("explaintext", "1"),
                ("redirects", "1"),
                ("titles", title),
            ],
        );
        let v = self.get_json(&url).await?;
        let text = v
            .pointer("/query/pages/0/extract")
            .and_then(Value::as_str)
            .map(clean_text)
            .filter(|t| !t.is_empty());
        Ok(text)
    }

    pub fn article_url(&self, title: &str) -> String {
        let path = title.replace(' ', "_");
        let enc = percent_encoding::utf8_percent_encode(&path, percent_encoding::NON_ALPHANUMERIC)
            .to_string()
            .replace("%5F", "_");
        format!("https://{}.wikipedia.org/wiki/{enc}", self.lang)
    }

    /// Look the artist up; `mbid` from Plex when it has one.
    pub async fn lookup(&self, mbid: Option<&str>, name: &str) -> Fetch<WikiData> {
        let mut entity: Option<(String, Value)> = None;
        if let Some(m) = mbid.filter(|m| is_mbid(m))
            && let Some(qid) = self.qid_by_mbid(m).await?
        {
            let ents = self.entities(std::slice::from_ref(&qid), "claims|sitelinks").await?;
            if let Some(e) = ents.get(&qid) {
                entity = Some((qid, e.clone()));
            }
        }
        if entity.is_none() && !name.trim().is_empty() {
            entity = self.qid_by_name(name).await?;
        }
        let Some((qid, e)) = entity else { return Ok(None) };

        // Labels for origin and members in one call.
        let origin_ids: Vec<String> = if is_group(&e) {
            [claim_ids(&e, "P740"), claim_ids(&e, "P495")].concat()
        } else {
            [claim_ids(&e, "P19"), claim_ids(&e, "P27")].concat()
        };
        let origin_id = origin_ids.into_iter().next();
        let member_ids: Vec<String> = if is_group(&e) {
            claim_ids(&e, "P527").into_iter().take(MAX_MEMBERS).collect()
        } else {
            Vec::new()
        };
        let mut want: Vec<String> = member_ids.clone();
        want.extend(origin_id.clone());
        let labels = if want.is_empty() { HashMap::new() } else { self.labels(&want).await? };

        let (start, end) = if is_group(&e) {
            (claim_year(&e, "P2031").or(claim_year(&e, "P571")), claim_year(&e, "P2032").or(claim_year(&e, "P576")))
        } else {
            (claim_year(&e, "P2031"), claim_year(&e, "P2032"))
        };

        let title = e
            .pointer(&format!("/sitelinks/{}wiki/title", self.lang))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let bio = match &title {
            Some(t) => self.extract(t).await?,
            None => None,
        };

        Ok(Some(WikiData {
            qid,
            bio,
            article_url: title.as_deref().map(|t| self.article_url(t)),
            origin: origin_id.and_then(|q| labels.get(&q).cloned()),
            years_active: years_active(start, end),
            members: member_ids.iter().filter_map(|q| labels.get(q).cloned()).collect(),
            mbid: claim_string(&e, "P434").filter(|m| is_mbid(m)),
        }))
    }

    async fn labels(&self, ids: &[String]) -> Result<HashMap<String, String>, FetchError> {
        let mut out = HashMap::new();
        // The API takes 50 ids per call.
        for chunk in ids.chunks(50) {
            let ents = self.entities(chunk, "labels").await?;
            for id in chunk {
                let label = ents
                    .pointer(&format!("/{id}/labels/{}/value", self.lang))
                    .or_else(|| ents.pointer(&format!("/{id}/labels/mul/value")))
                    .and_then(Value::as_str);
                if let Some(l) = label {
                    out.insert(id.clone(), l.to_owned());
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_years_active() {
        assert_eq!(years_active(Some(1985), None).as_deref(), Some("1985–present"));
        assert_eq!(years_active(Some(1964), Some(2009)).as_deref(), Some("1964–2009"));
        assert_eq!(years_active(None, None), None);
    }

    #[test]
    fn article_urls_use_underscores() {
        let l = Limiter::per_second(5.0);
        let http = super::super::http::fake::FakeHttp::new();
        let w = Wiki { http: &http, limiter: &l, lang: "en" };
        assert_eq!(w.article_url("Sigur Rós"), "https://en.wikipedia.org/wiki/Sigur_R%C3%B3s");
    }
}
