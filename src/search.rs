//! Search (KEYBINDS.md "Search contents").
//!
//! A strict search matches the query exactly. A permissive search also
//! allows:
//! - English: case-insensitive matching, and full-width ＡＢＣ matches ABC.
//! - Korean: 초성 matching, so ㅎㄱ finds 한국. A trailing syllable without
//!   a final consonant matches any final, so 한구 already finds 한국 while
//!   you are still typing.
//! - Japanese: ひらがな and カタカナ match each other, and a kana query is
//!   converted into kanji candidates (しんじゅく finds 新宿) using the
//!   dictionary from the `inputx-nihongo` crate (MIT OR Apache-2.0).
//!
//! Hangul handling is plain arithmetic on the Unicode syllable block and
//! needs no dependency.

use std::collections::HashMap;
use std::sync::OnceLock;

use unicode_segmentation::UnicodeSegmentation;

/// Stop collecting matches past this many, to keep huge files responsive.
const MAX_MATCHES: usize = 10_000;
/// Kanji candidates tried per kana query, most frequent first.
const MAX_CANDIDATES: usize = 12;

/// A match on one line, as a grapheme range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Match {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// One position in a search pattern.
#[derive(Clone, Copy, Debug)]
enum Atom {
    Char(char),
    /// A bare initial consonant (ㄱ, ㄴ, …): matches any syllable that
    /// starts with it, or the jamo itself.
    Initial(u32),
    /// A syllable without a final consonant: matches it with any final.
    Open {
        cho: u32,
        jung: u32,
    },
}

pub struct Matcher {
    exact: bool,
    patterns: Vec<Vec<Atom>>,
    /// Kanji spellings found for a kana query, for display.
    pub candidates: Vec<String>,
}

impl Matcher {
    pub fn new(query: &str, exact: bool) -> Self {
        if exact {
            return Self {
                exact,
                patterns: vec![query.chars().map(Atom::Char).collect()],
                candidates: Vec::new(),
            };
        }
        let candidates = kanji_candidates(query);
        let mut patterns = vec![permissive_atoms(query)];
        patterns.extend(
            candidates
                .iter()
                .map(|c| c.chars().map(|ch| Atom::Char(fold(ch))).collect()),
        );
        Self {
            exact,
            patterns,
            candidates,
        }
    }

    /// Every non-overlapping match in the buffer, in document order.
    /// Where several patterns match at one spot, the longest wins.
    pub fn find_all(&self, lines: &[String]) -> Vec<Match> {
        let mut out = Vec::new();
        for (row, line) in lines.iter().enumerate() {
            let (chars, gidx) = prepare(line, self.exact);
            let mut i = 0;
            while i < chars.len() {
                let best = self
                    .patterns
                    .iter()
                    .filter_map(|p| match_at(p, &chars, i))
                    .max();
                let Some(len) = best else {
                    i += 1;
                    continue;
                };
                out.push(Match {
                    row,
                    start: gidx[i],
                    end: gidx[i + len - 1] + 1,
                });
                if out.len() >= MAX_MATCHES {
                    return out;
                }
                i += len;
            }
        }
        out
    }
}

/// The line as (possibly folded) chars, each tagged with the index of the
/// grapheme it belongs to.
fn prepare(line: &str, exact: bool) -> (Vec<char>, Vec<usize>) {
    let mut chars = Vec::with_capacity(line.len());
    let mut gidx = Vec::with_capacity(line.len());
    for (gi, g) in line.graphemes(true).enumerate() {
        for c in g.chars() {
            chars.push(if exact { c } else { fold(c) });
            gidx.push(gi);
        }
    }
    (chars, gidx)
}

fn match_at(pattern: &[Atom], chars: &[char], start: usize) -> Option<usize> {
    if pattern.is_empty() || start + pattern.len() > chars.len() {
        return None;
    }
    pattern
        .iter()
        .zip(&chars[start..])
        .all(|(a, &c)| atom_matches(*a, c))
        .then_some(pattern.len())
}

fn atom_matches(atom: Atom, c: char) -> bool {
    match atom {
        Atom::Char(q) => q == c,
        Atom::Initial(i) => {
            c == CHOSEONG[i as usize] || decompose(c).is_some_and(|(cho, _, _)| cho == i)
        }
        Atom::Open { cho, jung } => decompose(c).is_some_and(|(a, b, _)| a == cho && b == jung),
    }
}

fn permissive_atoms(query: &str) -> Vec<Atom> {
    let chars: Vec<char> = query.chars().map(fold).collect();
    let last = chars.len().saturating_sub(1);
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            if let Some(idx) = CHOSEONG.iter().position(|&j| j == c) {
                Atom::Initial(idx as u32)
            } else if let Some((cho, jung, 0)) = decompose(c).filter(|_| i == last) {
                Atom::Open { cho, jung }
            } else {
                Atom::Char(c)
            }
        })
        .collect()
}

/// Normalize a char for permissive comparison: lowercase, full-width ASCII
/// to half-width, katakana to hiragana.
fn fold(c: char) -> char {
    let c = match c {
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
        _ => c,
    };
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

// ----- Korean ---------------------------------------------------------------

/// The 19 initial consonants, as compatibility jamo (what keyboards type).
const CHOSEONG: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];

/// Split a precomposed Hangul syllable into (initial, medial, final)
/// indices. Final 0 means no final consonant.
fn decompose(c: char) -> Option<(u32, u32, u32)> {
    let s = (c as u32).checked_sub(0xAC00).filter(|&s| s < 11_172)?;
    Some((s / 588, (s % 588) / 28, s % 28))
}

// ----- Japanese -------------------------------------------------------------

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | 'ー')
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}' | '々')
}

/// Kanji spellings for a query made only of kana (two or more, so a
/// single か doesn't light up every kanji read "ka").
fn kanji_candidates(query: &str) -> Vec<String> {
    let kana: String = query.chars().map(fold).collect();
    if kana.chars().count() < 2 || !kana.chars().all(is_kana) {
        return Vec::new();
    }
    kana_index().get(&kana).cloned().unwrap_or_default()
}

/// Hiragana reading → kanji spellings, most frequent first. Built once from
/// inputx-nihongo's romaji-keyed tables on the first kana search.
fn kana_index() -> &'static HashMap<String, Vec<String>> {
    static INDEX: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    INDEX.get_or_init(|| {
        use inputx_nihongo::{jukugo, kanji, romaji};

        let mut scored: HashMap<String, Vec<(String, u32)>> = HashMap::new();
        let mut add = |reading: &str, surface: String, freq: u32| {
            let kana = romaji::to_hiragana(reading);
            if kana.chars().all(is_kana) && surface.chars().any(is_han) {
                scored.entry(kana).or_default().push((surface, freq));
            }
        };
        for e in jukugo::JUKUGO_TABLE {
            add(e.reading, e.kanji.to_string(), e.freq);
        }
        for e in kanji::KANJI_TABLE {
            for r in e.readings {
                add(r, e.kanji.to_string(), e.freq);
            }
        }

        scored
            .into_iter()
            .map(|(kana, mut list)| {
                list.sort_by_key(|e| std::cmp::Reverse(e.1));
                let mut seen = Vec::new();
                for (surface, _) in list {
                    if !seen.contains(&surface) && seen.len() < MAX_CANDIDATES {
                        seen.push(surface);
                    }
                }
                (kana, seen)
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(query: &str, exact: bool, text: &str) -> Vec<(usize, usize, usize)> {
        let lines: Vec<String> = text.split('\n').map(String::from).collect();
        Matcher::new(query, exact)
            .find_all(&lines)
            .into_iter()
            .map(|m| (m.row, m.start, m.end))
            .collect()
    }

    #[test]
    fn english_is_case_insensitive_and_width_insensitive() {
        assert_eq!(
            find("hello", false, "Say HELLO, ｈｅｌｌｏ"),
            [(0, 4, 9), (0, 11, 16)]
        );
        assert!(find("hello", true, "Say HELLO").is_empty());
    }

    #[test]
    fn korean_initials() {
        assert_eq!(find("ㅎㄱ", false, "대한민국 한국어"), [(0, 5, 7)]);
        assert_eq!(find("ㄷㅎㅁㄱ", false, "대한민국"), [(0, 0, 4)]);
        assert_eq!(
            find("한ㄱ", false, "한국 한글 하고"),
            [(0, 0, 2), (0, 3, 5)]
        );
        assert!(find("ㅎㄱ", true, "한국").is_empty());
    }

    #[test]
    fn korean_open_last_syllable() {
        assert_eq!(find("한구", false, "한국어"), [(0, 0, 2)]);
        // Only the last syllable is open; 하 in the middle must be exact.
        assert!(find("하국", false, "한국").is_empty());
    }

    #[test]
    fn kana_scripts_match_each_other() {
        assert_eq!(find("てすと", false, "テスト中"), [(0, 0, 3)]);
        assert_eq!(find("テスト", false, "てすと"), [(0, 0, 3)]);
    }

    #[test]
    fn kana_finds_kanji_candidates() {
        let m = Matcher::new("しんじゅく", false);
        assert!(m.candidates.iter().any(|c| c == "新宿"));
        assert_eq!(find("しんじゅく", false, "新宿駅",), [(0, 0, 2)]);
        assert_eq!(
            find("がっこう", false, "学校とがっこう"),
            [(0, 0, 2), (0, 3, 7)]
        );
        assert!(find("しんじゅく", true, "新宿").is_empty());
    }

    #[test]
    fn exact_search_is_literal() {
        assert_eq!(find("日本", true, "日本と日本語"), [(0, 0, 2), (0, 3, 5)]);
    }
}
