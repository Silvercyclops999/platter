//! Telling plex.tv where this computer's control API answers, the way Plex
//! players publish their addresses. The user's other devices then see this
//! computer in their plex.tv resources (with its addresses and when it was
//! last seen), which finds it when mDNS doesn't cross the network and tells
//! "off" apart from "on but unreachable".

use url::Url;

use crate::error::AppResult;
use crate::plex::Token;
use crate::plex::client::{check_status, map_send_error};

/// How often the addresses are published while announcing (keeps plex.tv's
/// last seen time fresh).
pub const EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// The publish URL: `PUT /devices/<client id>?Connection[][uri]=...`.
pub fn publish_url(plex_tv: &Url, client_id: &str, bases: &[Url]) -> AppResult<Url> {
    let mut u = plex_tv
        .join(&format!("/devices/{}", urlencode(client_id)))
        .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
    {
        let mut q = u.query_pairs_mut();
        for b in bases {
            q.append_pair("Connection[][uri]", b.as_str().trim_end_matches('/'));
        }
    }
    Ok(u)
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Publish the addresses (the http client carries Platter's identity headers).
pub async fn publish(http: &reqwest::Client, plex_tv: &Url, token: &Token, client_id: &str, bases: &[Url]) -> AppResult<()> {
    let url = publish_url(plex_tv, client_id, bases)?;
    let resp = http
        .put(url)
        .header("x-plex-token", token.header_value()?)
        .send()
        .await
        .map_err(map_send_error)?;
    check_status(resp).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_url_lists_each_address() {
        let u = publish_url(
            &Url::parse("https://plex.tv").unwrap(),
            "abc-1",
            &[Url::parse("http://192.0.2.5:32520/").unwrap(), Url::parse("http://192.0.2.6:32520").unwrap()],
        )
        .unwrap();
        assert_eq!(u.path(), "/devices/abc-1");
        let pairs: Vec<(String, String)> = u.query_pairs().into_owned().collect();
        assert_eq!(
            pairs,
            [
                ("Connection[][uri]".to_string(), "http://192.0.2.5:32520".to_string()),
                ("Connection[][uri]".to_string(), "http://192.0.2.6:32520".to_string())
            ]
        );
    }
}
