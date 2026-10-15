//! Pronunciation lines for lyrics: a romanised line under each line written
//! in Japanese, Korean, Chinese, Hindi (Devanagari) or Cyrillic. All local.
//!
//! - Japanese (kana and kanji): the `kakasi` crate (GPL-3.0, KAKASI's
//!   dictionary), Hepburn.
//! - Chinese: the `pinyin` crate (MIT), pinyin with tone marks, one
//!   reading per character.
//! - Korean: Revised Romanisation worked out from the Hangul syllables,
//!   with the final consonant carried over to a silent next syllable.
//! - Hindi: a plain-letter table (no diacritics), dropping the vowel a
//!   Devanagari consonant carries at the end of a word.
//! - Cyrillic: a plain-letter table covering Russian, Ukrainian,
//!   Belarusian, Serbian and Macedonian letters.
//!
//! Kanji with no kana anywhere in the song read as Chinese.

use pinyin::ToPinyin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    Hangul,
    Cyrillic,
    Devanagari,
    /// Kana and CJK ideographs (and the CJK punctuation between them).
    Cjk,
    Other,
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}')
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}' | '\u{3005}' | '\u{3007}')
}

fn script(c: char) -> Script {
    match c {
        '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' => Script::Hangul,
        '\u{0400}'..='\u{04FF}' => Script::Cyrillic,
        '\u{0900}'..='\u{097F}' => Script::Devanagari,
        c if is_kana(c) || is_han(c) || matches!(c, '\u{3000}'..='\u{303F}' | '\u{FF01}'..='\u{FF5E}') => Script::Cjk,
        _ => Script::Other,
    }
}

/// Whether a line has anything to romanise.
fn needs(line: &str) -> bool {
    line.chars().any(|c| match script(c) {
        Script::Other => false,
        // CJK punctuation alone isn't worth a line.
        Script::Cjk => is_kana(c) || is_han(c),
        _ => true,
    })
}

/// One pronunciation line per lyrics line (None where the line has nothing
/// to romanise, like English lines or instrumental gaps).
pub fn pronounce(lines: &[String]) -> Vec<Option<String>> {
    let japanese = lines.iter().any(|l| l.chars().any(is_kana));
    lines.iter().map(|l| needs(l).then(|| line(l, japanese))).collect()
}

fn line(text: &str, japanese: bool) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let mut kind = Script::Other;
    let flush = |out: &mut String, run: &mut String, kind: Script| {
        if run.is_empty() {
            return;
        }
        let piece = match kind {
            Script::Hangul => korean(run),
            Script::Cyrillic => cyrillic(run),
            Script::Devanagari => hindi(run),
            Script::Cjk if japanese => kakasi::convert(run.as_str()).romaji,
            Script::Cjk => chinese(run),
            Script::Other => run.clone(),
        };
        // Keep words apart where scripts meet.
        if !out.is_empty() && !out.ends_with(' ') && kind != Script::Other && piece.starts_with(|c: char| c.is_alphanumeric()) {
            out.push(' ');
        }
        out.push_str(&piece);
        run.clear();
    };
    for c in text.chars() {
        let s = script(c);
        // Spaces and Latin text stay in a Japanese run so kakasi sees the
        // whole phrase.
        let s = if japanese && kind == Script::Cjk && s == Script::Other && !c.is_ascii_alphabetic() { Script::Cjk } else { s };
        if s != kind {
            flush(&mut out, &mut run, kind);
            kind = s;
        }
        run.push(c);
    }
    flush(&mut out, &mut run, kind);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------ Chinese

fn cjk_punct(c: char) -> Option<&'static str> {
    Some(match c {
        '，' | '、' => ",",
        '。' => ".",
        '！' => "!",
        '？' => "?",
        '：' => ":",
        '；' => ";",
        '（' => "(",
        '）' => ")",
        '「' | '」' | '『' | '』' | '“' | '”' => "\"",
        '…' => "...",
        '\u{3000}' => " ",
        _ => return None,
    })
}

fn chinese(run: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for c in run.chars() {
        if let Some(p) = c.to_pinyin() {
            words.push(p.with_tone().to_string());
        } else if let Some(p) = cjk_punct(c) {
            match words.last_mut() {
                Some(w) if p != " " && p != "(" => w.push_str(p),
                _ => words.push(p.into()),
            }
        } else if !c.is_whitespace() {
            words.push(c.to_string());
        }
    }
    words.join(" ")
}

// ------------------------------------------------------------ Korean

const ONSET: [&str; 19] = ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"];
const VOWEL: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui", "i",
];
/// A final consonant at the end of a syllable.
const CODA: [&str; 28] = [
    "", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "l", "l", "l", "p", "l", "m", "p", "p", "t", "t", "ng", "t", "t", "k", "t", "p", "t",
];
/// A final consonant carried onto a next syllable that starts silent:
/// (what stays, what moves).
const CARRY: [(&str, &str); 28] = [
    ("", ""),
    ("", "g"),
    ("", "kk"),
    ("k", "s"),
    ("", "n"),
    ("n", "j"),
    ("n", ""),
    ("", "d"),
    ("", "r"),
    ("l", "g"),
    ("l", "m"),
    ("l", "b"),
    ("l", "s"),
    ("l", "t"),
    ("l", "p"),
    ("l", ""),
    ("", "m"),
    ("", "b"),
    ("p", "s"),
    ("", "s"),
    ("", "ss"),
    ("ng", ""),
    ("", "j"),
    ("", "ch"),
    ("", "k"),
    ("", "t"),
    ("", "p"),
    ("", ""),
];

fn korean(run: &str) -> String {
    let parts: Vec<Option<(usize, usize, usize)>> = run
        .chars()
        .map(|c| {
            let s = (c as u32).checked_sub(0xAC00).filter(|s| *s < 11_172)? as usize;
            Some((s / 588, (s % 588) / 28, s % 28))
        })
        .collect();
    let chars: Vec<char> = run.chars().collect();
    let mut out = String::new();
    // An onset already written by the syllable before (carried over).
    let mut carried: Option<&str> = None;
    for (i, p) in parts.iter().enumerate() {
        let Some((l, v, t)) = *p else {
            out.push(chars[i]);
            carried = None;
            continue;
        };
        match carried.take() {
            Some(on) => out.push_str(on),
            None => out.push_str(ONSET[l]),
        }
        out.push_str(VOWEL[v]);
        let next_silent = matches!(parts.get(i + 1), Some(Some((11, _, _))));
        if t != 0 && next_silent {
            let (stay, mv) = CARRY[t];
            out.push_str(stay);
            carried = Some(mv);
        } else if t == 8 && matches!(parts.get(i + 1), Some(Some((5, _, _)))) {
            // ㄹ before ㄹ reads ll.
            out.push('l');
            carried = Some("l");
        } else {
            out.push_str(CODA[t]);
        }
    }
    out
}

// ------------------------------------------------------------ Hindi

fn deva_consonant(c: char) -> Option<&'static str> {
    Some(match c {
        'क' => "k",
        'ख' => "kh",
        'ग' => "g",
        'घ' => "gh",
        'ङ' => "n",
        'च' => "ch",
        'छ' => "chh",
        'ज' => "j",
        'झ' => "jh",
        'ञ' => "n",
        'ट' => "t",
        'ठ' => "th",
        'ड' => "d",
        'ढ' => "dh",
        'ण' => "n",
        'त' => "t",
        'थ' => "th",
        'द' => "d",
        'ध' => "dh",
        'न' | 'ऩ' => "n",
        'प' => "p",
        'फ' => "ph",
        'ब' => "b",
        'भ' => "bh",
        'म' => "m",
        'य' => "y",
        'र' | 'ऱ' => "r",
        'ल' | 'ळ' | 'ऴ' => "l",
        'व' => "v",
        'श' | 'ष' => "sh",
        'स' => "s",
        'ह' => "h",
        '\u{0958}' => "q",
        '\u{0959}' => "kh",
        '\u{095A}' => "gh",
        '\u{095B}' => "z",
        '\u{095C}' => "r",
        '\u{095D}' => "rh",
        '\u{095E}' => "f",
        '\u{095F}' => "y",
        _ => return None,
    })
}

/// A consonant with a nukta under it.
fn nukta(c: &str) -> &'static str {
    match c {
        "k" => "q",
        "kh" => "kh",
        "g" => "gh",
        "j" => "z",
        "d" => "r",
        "dh" => "rh",
        "ph" => "f",
        "y" => "y",
        "r" => "r",
        "l" => "l",
        _ => "n",
    }
}

fn deva_vowel(c: char) -> Option<&'static str> {
    Some(match c {
        'अ' => "a",
        'आ' | 'ा' => "aa",
        'इ' | 'ि' => "i",
        'ई' | 'ी' => "ee",
        'उ' | 'ु' => "u",
        'ऊ' | 'ू' => "oo",
        'ऋ' | 'ृ' => "ri",
        'ए' | 'े' => "e",
        'ऐ' | 'ै' => "ai",
        'ओ' | 'ो' | 'ऑ' | 'ॉ' => "o",
        'औ' | 'ौ' => "au",
        'ऍ' | 'ॅ' | 'ऎ' | 'ॆ' => "e",
        'ऒ' | 'ॊ' => "o",
        _ => return None,
    })
}

fn is_matra(c: char) -> bool {
    matches!(c, '\u{093E}'..='\u{094C}' | '\u{0962}' | '\u{0963}')
}

fn hindi(run: &str) -> String {
    let mut out = String::new();
    for word in run.split_inclusive(char::is_whitespace) {
        out.push_str(&hindi_word(word));
    }
    out
}

fn hindi_word(word: &str) -> String {
    let chars: Vec<char> = word.chars().collect();
    let mut out = String::new();
    // A consonant waiting for its vowel.
    let mut pending: Option<&'static str> = None;
    let last_letter = chars.iter().rposition(|c| deva_consonant(*c).is_some() || deva_vowel(*c).is_some());
    let push_vowel = |out: &mut String, v: &str, at: usize| {
        // A long vowel ending the word reads short (mera, ki).
        let v = if Some(at) == last_letter {
            match v {
                "aa" => "a",
                "ee" => "i",
                "oo" => "u",
                v => v,
            }
        } else {
            v
        };
        out.push_str(v);
    };
    for (i, &c) in chars.iter().enumerate() {
        if c == '\u{093C}' {
            // Nukta: changes the consonant before it.
            if let Some(p) = pending.as_mut() {
                *p = nukta(p);
            }
            continue;
        }
        if let Some(p) = pending.take() {
            out.push_str(p);
            if is_matra(c) {
                push_vowel(&mut out, deva_vowel(c).unwrap_or(""), i);
                continue;
            }
            if c == '\u{094D}' {
                continue;
            }
            // The vowel every consonant carries, dropped at the end of a word.
            let ends = chars[i..].iter().all(|x| !(deva_consonant(*x).is_some() || deva_vowel(*x).is_some()));
            if !ends {
                out.push('a');
            }
        }
        if let Some(k) = deva_consonant(c) {
            pending = Some(k);
        } else if let Some(v) = deva_vowel(c) {
            push_vowel(&mut out, v, i);
        } else {
            match c {
                'ं' | 'ँ' => out.push('n'),
                'ः' => out.push('h'),
                '।' | '॥' => out.push('.'),
                'ॐ' => out.push_str("om"),
                '०'..='९' => out.push(char::from(b'0' + (c as u32 - '०' as u32) as u8)),
                c if script(c) == Script::Devanagari => {}
                c => out.push(c),
            }
        }
    }
    if let Some(p) = pending {
        out.push_str(p);
    }
    out
}

// ------------------------------------------------------------ Cyrillic

fn cyr(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "yo",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' => "",
        'ы' => "y",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        'є' => "ye",
        'і' => "i",
        'ї' => "yi",
        'ґ' => "g",
        'ў' => "u",
        'ђ' => "dj",
        'ј' => "j",
        'љ' => "lj",
        'њ' => "nj",
        'ћ' => "c",
        'џ' => "dz",
        'ѓ' => "gj",
        'ќ' => "kj",
        'ѕ' => "dz",
        _ => return None,
    })
}

fn cyrillic(run: &str) -> String {
    let mut out = String::new();
    for c in run.chars() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        match cyr(lower) {
            Some(s) if lower != c => {
                let mut cs = s.chars();
                if let Some(f) = cs.next() {
                    out.extend(f.to_uppercase());
                    out.push_str(cs.as_str());
                }
            }
            Some(s) => out.push_str(s),
            None if c == '\u{2019}' || c == '\'' => {}
            None => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Option<String> {
        pronounce(&[s.to_string()]).remove(0)
    }

    #[test]
    fn japanese_kana_and_kanji_read_in_hepburn() {
        assert_eq!(one("こんにちは世界").as_deref(), Some("konnichiha sekai"));
        let got = pronounce(&["夜に駆ける".into(), "沈むように溶けてゆくように".into(), "Hello".into()]);
        assert!(got[0].as_deref().is_some_and(|s| s.starts_with("yoru")), "{got:?}");
        assert!(got[1].as_deref().is_some_and(|s| s.contains("shizumu") && s.contains("you")), "{got:?}");
        assert_eq!(got[2], None, "English lines get no line under them");
    }

    #[test]
    fn kanji_without_kana_in_the_song_read_as_chinese() {
        let got = pronounce(&["我爱你，中国".into(), "".into()]);
        assert_eq!(got[0].as_deref(), Some("wǒ ài nǐ, zhōng guó"));
        assert_eq!(got[1], None);
        // The same kanji in a song with kana anywhere read as Japanese.
        let jp = pronounce(&["中国".into(), "ありがとう".into()]);
        assert_eq!(jp[0].as_deref(), Some("chuugoku"));
        assert_eq!(jp[1].as_deref(), Some("arigatou"));
    }

    #[test]
    fn korean_revised_romanisation_with_carried_finals() {
        assert_eq!(one("안녕하세요").as_deref(), Some("annyeonghaseyo"));
        assert_eq!(one("사랑해").as_deref(), Some("saranghae"));
        assert_eq!(one("한국어").as_deref(), Some("hangugeo"));
        assert_eq!(one("서울 빛나").as_deref(), Some("seoul bitna"));
        assert_eq!(one("몰라").as_deref(), Some("molla"));
        assert_eq!(one("I love you 사랑해!").as_deref(), Some("I love you saranghae!"));
    }

    #[test]
    fn hindi_drops_the_final_vowel_and_shortens_long_ends() {
        assert_eq!(one("मेरा दिल").as_deref(), Some("mera dil"));
        assert_eq!(one("तुम ही हो").as_deref(), Some("tum hi ho"));
        assert_eq!(one("प्यार").as_deref(), Some("pyaar"));
        assert_eq!(one("ज़िंदगी").as_deref(), Some("zindagi"));
        assert_eq!(one("नमस्ते।").as_deref(), Some("namaste."));
    }

    #[test]
    fn cyrillic_letters_keep_their_case() {
        assert_eq!(one("Привет, мир").as_deref(), Some("Privet, mir"));
        assert_eq!(one("Щастя їжак").as_deref(), Some("Shchastya yizhak"));
        assert_eq!(one("Љубав").as_deref(), Some("Ljubav"));
    }

    #[test]
    fn nothing_to_romanise_gives_none() {
        assert_eq!(pronounce(&["Just English".into(), "".into(), "「」".into()]), vec![None, None, None]);
    }
}
