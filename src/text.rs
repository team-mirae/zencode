//! Display-width helpers. Everything on screen is measured in terminal
//! columns, and zenkaku (full-width) characters such as 한글 and 日本語 take
//! two columns each. The cursor always moves by grapheme cluster, never by
//! byte or `char`, so combining marks and emoji sequences stay intact.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

static TAB_WIDTH: AtomicUsize = AtomicUsize::new(4);

/// Columns between tab stops; `-T` / `--tabsize` changes it at startup.
pub fn tab_width() -> usize {
    TAB_WIDTH.load(Ordering::Relaxed)
}

pub fn set_tab_width(n: usize) {
    TAB_WIDTH.store(n.max(1), Ordering::Relaxed);
}

/// Some East Asian terminal setups render "ambiguous width" characters
/// (·, ─, →, ①, …) as two columns. Set `ZENCODE_AMBIGUOUS_WIDE=1` to match.
fn ambiguous_wide() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        std::env::var("ZENCODE_AMBIGUOUS_WIDE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false)
    })
}

pub fn is_control(g: &str) -> bool {
    g != "\t" && g.chars().any(char::is_control)
}

/// Width of a grapheme that starts at display column `col`.
/// `col` only matters for tabs, which expand to the next tab stop.
pub fn grapheme_width(g: &str, col: usize) -> usize {
    if g == "\t" {
        let tab = tab_width();
        return tab - col % tab;
    }
    if is_control(g) {
        return 1;
    }
    if ambiguous_wide() {
        g.width_cjk()
    } else {
        g.width()
    }
}

/// Total display width of `s`, assuming it starts at column 0.
pub fn str_width(s: &str) -> usize {
    let mut col = 0;
    for g in s.graphemes(true) {
        col += grapheme_width(g, col);
    }
    col
}

pub fn grapheme_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// Byte offset of the `gidx`-th grapheme (or `s.len()` past the end).
pub fn byte_index(s: &str, gidx: usize) -> usize {
    s.grapheme_indices(true)
        .nth(gidx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Display column at which the `gidx`-th grapheme starts.
pub fn display_col(s: &str, gidx: usize) -> usize {
    let mut col = 0;
    for g in s.graphemes(true).take(gidx) {
        col += grapheme_width(g, col);
    }
    col
}

/// Width of the grapheme at `gidx`, or 1 when the cursor sits past the end.
pub fn width_at(s: &str, gidx: usize) -> usize {
    let col = display_col(s, gidx);
    s.graphemes(true)
        .nth(gidx)
        .map(|g| grapheme_width(g, col).max(1))
        .unwrap_or(1)
}

/// Grapheme index whose cell covers display column `target`; used to keep
/// the cursor in roughly the same column when moving between lines that
/// mix half-width and full-width text.
pub fn gidx_for_col(s: &str, target: usize) -> usize {
    let mut col = 0;
    for (i, g) in s.graphemes(true).enumerate() {
        let w = grapheme_width(g, col);
        if col + w > target {
            return i;
        }
        col += w;
    }
    grapheme_count(s)
}

/// Truncate `s` so it fits within `max` columns, never splitting a
/// full-width character in half.
pub fn truncate(s: &str, max: usize) -> (&str, usize) {
    let mut col = 0;
    for (i, g) in s.grapheme_indices(true) {
        let w = grapheme_width(g, col);
        if col + w > max {
            return (&s[..i], col);
        }
        col += w;
    }
    (s, col)
}

fn is_blank(g: &str) -> bool {
    matches!(g, " " | "\t" | "\u{3000}")
}

fn is_hangul(g: &str) -> bool {
    g.chars().next().is_some_and(|c| {
        matches!(c, '\u{AC00}'..='\u{D7A3}' | '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}')
    })
}

/// Characters that may not start a wrapped row (kinsoku): closing
/// brackets, punctuation and small kana.
fn no_row_start(g: &str) -> bool {
    g.chars().next().is_some_and(|c| {
        "、。，．・：；？！ー」』）］｝〉》】〕〙〗ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ々〜…‥),.!?:;]}".contains(c)
    })
}

/// Characters that may not end a wrapped row: opening brackets.
fn no_row_end(g: &str) -> bool {
    g.chars()
        .next()
        .is_some_and(|c| "「『（［｛〈《【〔〘〖([{".contains(c))
}

/// Whether a wrapped row may start at `g` when `prev` comes before it.
/// Rows break after blanks, and on either side of full-width text such
/// as 日本語 that has no spaces to break at. 한글 is written with spaces,
/// so it only breaks at those, keeping words whole.
fn can_break(prev: &str, g: &str) -> bool {
    if is_blank(prev) {
        return !is_blank(g);
    }
    if is_blank(g) {
        return true;
    }
    let cjk = |g: &str| grapheme_width(g, 0) > 1 && !is_hangul(g);
    (cjk(prev) || cjk(g))
        && !is_hangul(prev)
        && !is_hangul(g)
        && !no_row_start(g)
        && !no_row_end(prev)
}

/// Where one screen row of a soft-wrapped line starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Index of the row's first grapheme in the line.
    pub start: usize,
    /// Display column of that grapheme in the unwrapped line.
    pub col: usize,
    /// Byte offset of that grapheme, so work on the row can skip ahead.
    pub byte: usize,
}

impl Segment {
    pub const WHOLE: Segment = Segment {
        start: 0,
        col: 0,
        byte: 0,
    };

    /// Display column (in the unwrapped line) of grapheme `gidx`, which
    /// must be at or after the row's start.
    pub fn col_of(&self, line: &str, gidx: usize) -> usize {
        let mut col = self.col;
        for g in line[self.byte..].graphemes(true).take(gidx - self.start) {
            col += grapheme_width(g, col);
        }
        col
    }

    /// Grapheme from the row's start onwards whose cell covers display
    /// column `target`, like [`gidx_for_col`]; at most `limit`.
    pub fn gidx_at(&self, line: &str, target: usize, limit: usize) -> usize {
        let mut col = self.col;
        let mut gidx = self.start;
        for g in line[self.byte..].graphemes(true) {
            let w = grapheme_width(g, col);
            if gidx >= limit || col + w > target {
                break;
            }
            col += w;
            gidx += 1;
        }
        gidx.min(limit)
    }
}

/// Soft-wrap `s` into rows of at most `width` columns, breaking between
/// words where it can and anywhere inside a word too long for a row.
/// The first row always starts at 0. Tabs keep the widths they have in
/// the unwrapped line.
///
/// The cursor can sit after the last character, so a last row that fills
/// the width exactly gets an empty row after it for the cursor to go to.
pub fn wrap(s: &str, width: usize) -> Vec<Segment> {
    let width = width.max(1);
    let mut rows = vec![Segment::WHOLE];
    // Display column and byte offset where each grapheme starts, to look
    // back on a break.
    let mut cols = Vec::new();
    let mut col = 0;
    let mut row_start = Segment::WHOLE;
    let mut last_break = 0;
    // Whether the row has non-blank text before the last break
    // opportunity, before the current grapheme, and since that break.
    let mut words_before_break = false;
    let mut row_words = false;
    let mut words_since_break = false;
    let mut prev = "";
    for (i, (byte, g)) in s.grapheme_indices(true).enumerate() {
        let w = grapheme_width(g, col);
        cols.push((col, byte));
        if i > row_start.start && can_break(prev, g) {
            last_break = i;
            words_before_break = row_words;
            words_since_break = false;
        }
        if col + w - row_start.col > width && i > row_start.start {
            // Break at the last opportunity, unless it would leave this
            // row holding nothing but blanks.
            let at = if last_break > row_start.start && words_before_break {
                row_words = words_since_break;
                last_break
            } else {
                row_words = false;
                i
            };
            row_start = Segment {
                start: at,
                col: cols[at].0,
                byte: cols[at].1,
            };
            rows.push(row_start);
            last_break = at;
            words_since_break = row_words;
        }
        if !is_blank(g) {
            row_words = true;
            words_since_break = true;
        }
        col += w;
        prev = g;
    }
    if col - row_start.col >= width {
        rows.push(Segment {
            start: cols.len(),
            col,
            byte: s.len(),
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zenkaku_is_two_columns() {
        assert_eq!(str_width("abc"), 3);
        assert_eq!(str_width("한글"), 4);
        assert_eq!(str_width("日本語"), 6);
        assert_eq!(str_width("a한b日"), 6);
    }

    #[test]
    fn hangul_jamo_compose_into_one_grapheme() {
        // 한 written as decomposed jamo (ᄒ + ᅡ + ᆫ) is still one cell pair.
        let decomposed = "\u{1112}\u{1161}\u{11AB}";
        assert_eq!(grapheme_count(decomposed), 1);
        assert_eq!(str_width(decomposed), 2);
    }

    #[test]
    fn column_mapping_round_trips() {
        let s = "ab한글cd";
        assert_eq!(display_col(s, 2), 2);
        assert_eq!(display_col(s, 3), 4);
        assert_eq!(display_col(s, 4), 6);
        // Column 3 is the right half of 한, so it maps to 한 itself.
        assert_eq!(gidx_for_col(s, 3), 2);
        assert_eq!(gidx_for_col(s, 100), 6);
    }

    #[test]
    fn tabs_expand_to_stops() {
        assert_eq!(str_width("\t"), tab_width());
        assert_eq!(str_width("a\t"), tab_width());
        assert_eq!(str_width("한\tx"), tab_width() + 1);
    }

    #[test]
    fn truncate_never_splits_wide_chars() {
        assert_eq!(truncate("日本語", 5), ("日本", 4));
        assert_eq!(truncate("abc", 10), ("abc", 3));
    }
    fn rows(s: &str, width: usize) -> Vec<&str> {
        let starts: Vec<usize> = wrap(s, width).iter().map(|r| r.start).collect();
        let idx: Vec<usize> = s
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain([s.len()])
            .collect();
        starts
            .iter()
            .enumerate()
            .map(|(n, &g)| {
                let end = starts.get(n + 1).map_or(s.len(), |&e| idx[e]);
                &s[idx[g]..end]
            })
            .collect()
    }

    #[test]
    fn wrap_breaks_between_words() {
        assert_eq!(rows("hello world foo", 8), ["hello ", "world ", "foo"]);
        assert_eq!(rows("short", 80), ["short"]);
        assert_eq!(rows("", 10), [""]);
    }

    #[test]
    fn wrap_splits_long_words() {
        assert_eq!(rows("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_leaves_room_for_the_cursor() {
        assert_eq!(rows("abcd", 4), ["abcd", ""]);
    }

    #[test]
    fn wrap_keeps_korean_words_whole() {
        assert_eq!(rows("학교에서는 공부를", 12), ["학교에서는 ", "공부를"]);
    }

    #[test]
    fn wrap_breaks_japanese_anywhere_but_kinsoku() {
        // 。 can't start a row, so it goes with the character before it.
        assert_eq!(rows("あいう。えお", 6), ["あい", "う。え", "お"]);
        assert_eq!(rows("日本語", 4), ["日本", "語"]);
    }

    #[test]
    fn wrap_never_splits_wide_chars() {
        assert_eq!(rows("a日本", 4), ["a日", "本"]);
    }
}
