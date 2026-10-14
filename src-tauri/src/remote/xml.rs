//! A small reader for the flat XML Plex players and the Sonos relay answer
//! with (`<MediaContainer><Timeline .../></MediaContainer>`). It returns each
//! start or empty tag with its attributes; text and nesting are ignored,
//! which is all those answers carry.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

impl Element {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    pub fn int(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(|v| v.trim().parse::<f64>().ok()).map(|f| f as i64)
    }
}

/// Every start or empty-element tag, in document order.
pub fn elements(xml: &str) -> Vec<Element> {
    let b = xml.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = xml[i..].find('<') {
        i += off + 1;
        match b.get(i) {
            Some(b'/') | Some(b'?') | Some(b'!') | None => {
                // Closing tag, declaration, comment: skip past '>'.
                match xml[i..].find('>') {
                    Some(e) => i += e + 1,
                    None => break,
                }
                continue;
            }
            _ => {}
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
            i += 1;
        }
        let name = xml[start..i].to_string();
        let mut attrs = Vec::new();
        loop {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() {
                break;
            }
            if b[i] == b'>' {
                i += 1;
                break;
            }
            if b[i] == b'/' {
                i += 1;
                continue;
            }
            let ks = i;
            while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
                i += 1;
            }
            let key = xml[ks..i].to_string();
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if b.get(i) != Some(&b'=') {
                // A bare attribute (not valid XML); keep it empty.
                if !key.is_empty() {
                    attrs.push((key, String::new()));
                }
                continue;
            }
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let Some(&q) = b.get(i) else { break };
            if q != b'"' && q != b'\'' {
                break;
            }
            i += 1;
            let vs = i;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            attrs.push((key, unescape(&xml[vs..i.min(b.len())])));
            i += 1;
        }
        if !name.is_empty() {
            out.push(Element { name, attrs });
        }
        if i >= b.len() {
            break;
        }
    }
    out
}

/// XML-escape an attribute value.
pub fn escape(v: &str) -> String {
    let mut s = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            '\'' => s.push_str("&apos;"),
            c => s.push(c),
        }
    }
    s
}

fn unescape(v: &str) -> String {
    if !v.contains('&') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tags_and_attributes() {
        let x = r#"<?xml version="1.0" encoding="UTF-8"?>
<MediaContainer commandID="4" location="fullScreenMusic">
  <Timeline type="music" state="playing" time="61234" duration="200000" ratingKey="42" title="A &amp; B &#233;" />
  <Timeline type='video' state='stopped'/>
  <!-- note -->
</MediaContainer>"#;
        let els = elements(x);
        assert_eq!(els.len(), 3);
        assert_eq!(els[0].name, "MediaContainer");
        assert_eq!(els[0].int("commandID"), Some(4));
        assert_eq!(els[1].get("state"), Some("playing"));
        assert_eq!(els[1].int("time"), Some(61234));
        assert_eq!(els[1].get("title"), Some("A & B é"));
        assert_eq!(els[2].get("type"), Some("video"));
    }

    #[test]
    fn survives_junk() {
        assert!(elements("").is_empty());
        assert!(elements("plain text").is_empty());
        let e = elements("<Response code=\"200\" status=\"OK\"");
        assert_eq!(e[0].get("code"), Some("200"));
        assert_eq!(elements("<a b=\"1 & 2\">")[0].get("b"), Some("1 & 2"));
    }

    #[test]
    fn escape_round_trips() {
        let v = "Tom's \"Den\" <1> & co";
        let x = format!("<P n=\"{}\"/>", escape(v));
        assert_eq!(elements(&x)[0].get("n"), Some(v));
    }
}
