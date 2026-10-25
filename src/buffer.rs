//! The text being edited: a list of lines plus file metadata.
//! Positions are `(row, grapheme index)` pairs; ranges are half-open
//! `start..end` with `start <= end`.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use unicode_segmentation::UnicodeSegmentation;

use crate::text::{byte_index, grapheme_count};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

pub type Pos = (usize, usize);

pub struct Buffer {
    pub lines: Vec<String>,
    pub path: PathBuf,
    /// Bumped on every mutation, so callers can tell whether an edit
    /// actually changed anything.
    pub revision: u64,
    pub line_ending: LineEnding,
}

impl Buffer {
    /// Open `path`, creating it as an empty file first if it doesn't exist.
    /// Returns the buffer and whether the file was newly created.
    pub fn open(path: &Path) -> Result<(Self, bool)> {
        let created = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(_) => true,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => false,
            Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
        };
        if path.is_dir() {
            bail!("{} is a directory", path.display());
        }
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let Ok(content) = String::from_utf8(bytes) else {
            bail!("{} is not valid UTF-8", path.display());
        };
        let mut buf = Self::from_text(path.to_path_buf(), &content);
        if created {
            buf.line_ending = if cfg!(windows) {
                LineEnding::CrLf
            } else {
                LineEnding::Lf
            };
        }
        Ok((buf, created))
    }

    pub fn from_text(path: PathBuf, content: &str) -> Self {
        let line_ending = if content.contains("\r\n") {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        let mut lines: Vec<String> = content
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
            .collect();
        // A trailing newline terminates the last line rather than starting a new one.
        if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }

        Self {
            lines,
            path,
            revision: 0,
            line_ending,
        }
    }

    /// Write the buffer to its own path, returning the number of lines written.
    pub fn save(&self) -> Result<usize> {
        self.write_to(&self.path)
    }

    /// Write a copy of the buffer to `path` without rebinding the buffer.
    pub fn write_to(&self, path: &Path) -> Result<usize> {
        let eol = self.line_ending.as_str();
        let mut out = self.lines.join(eol);
        out.push_str(eol);
        fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
        Ok(self.lines.len())
    }

    pub fn end(&self) -> Pos {
        let last = self.lines.len() - 1;
        (last, self.line_len(last))
    }

    pub fn line_len(&self, row: usize) -> usize {
        grapheme_count(&self.lines[row])
    }

    /// Insert text (possibly spanning several lines) and return the
    /// position just after it.
    pub fn insert_str(&mut self, row: usize, gidx: usize, text: &str) -> (usize, usize) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let at = byte_index(&self.lines[row], gidx);
        let tail = self.lines[row].split_off(at);

        let mut parts = text.split('\n');
        self.lines[row].push_str(parts.next().unwrap_or(""));
        let mut r = row;
        for part in parts {
            r += 1;
            self.lines.insert(r, part.to_string());
        }
        let col = grapheme_count(&self.lines[r]);
        self.lines[r].push_str(&tail);
        self.revision += 1;
        (r, col)
    }

    /// Delete the grapheme before the cursor, joining lines at column 0.
    pub fn backspace(&mut self, row: usize, gidx: usize) -> (usize, usize) {
        if gidx > 0 {
            let line = &mut self.lines[row];
            let start = byte_index(line, gidx - 1);
            let end = byte_index(line, gidx);
            line.replace_range(start..end, "");
            self.revision += 1;
            (row, gidx - 1)
        } else if row > 0 {
            let line = self.lines.remove(row);
            let col = grapheme_count(&self.lines[row - 1]);
            self.lines[row - 1].push_str(&line);
            self.revision += 1;
            (row - 1, col)
        } else {
            (row, gidx)
        }
    }

    /// Delete the grapheme under the cursor, joining with the next line at EOL.
    pub fn delete(&mut self, row: usize, gidx: usize) {
        let line = &mut self.lines[row];
        if let Some((start, g)) = line.grapheme_indices(true).nth(gidx) {
            let end = start + g.len();
            line.replace_range(start..end, "");
            self.revision += 1;
        } else if row + 1 < self.lines.len() {
            let next = self.lines.remove(row + 1);
            self.lines[row].push_str(&next);
            self.revision += 1;
        }
    }

    /// Text between two positions, with lines joined by `\n`.
    pub fn text_range(&self, start: Pos, end: Pos) -> String {
        let (r0, c0) = start;
        let (r1, c1) = end;
        let b0 = byte_index(&self.lines[r0], c0);
        let b1 = byte_index(&self.lines[r1], c1);
        if r0 == r1 {
            return self.lines[r0][b0..b1].to_string();
        }
        let mut out = self.lines[r0][b0..].to_string();
        for line in &self.lines[r0 + 1..r1] {
            out.push('\n');
            out.push_str(line);
        }
        out.push('\n');
        out.push_str(&self.lines[r1][..b1]);
        out
    }

    /// Delete everything between two positions.
    pub fn delete_range(&mut self, start: Pos, end: Pos) {
        if start == end {
            return;
        }
        let (r0, c0) = start;
        let (r1, c1) = end;
        let b0 = byte_index(&self.lines[r0], c0);
        let b1 = byte_index(&self.lines[r1], c1);
        let tail = self.lines[r1][b1..].to_string();
        self.lines[r0].truncate(b0);
        self.lines[r0].push_str(&tail);
        self.lines.drain(r0 + 1..=r1);
        self.revision += 1;
    }

    /// Swap the block of lines `r0..=r1` with its neighbor above (`up`) or
    /// below. Returns false when the block is already at the edge.
    pub fn move_lines(&mut self, r0: usize, r1: usize, up: bool) -> bool {
        if up {
            if r0 == 0 {
                return false;
            }
            let line = self.lines.remove(r0 - 1);
            self.lines.insert(r1, line);
        } else {
            if r1 + 1 >= self.lines.len() {
                return false;
            }
            let line = self.lines.remove(r1 + 1);
            self.lines.insert(r0, line);
        }
        self.revision += 1;
        true
    }

    /// Remove a whole line, always leaving at least one line.
    pub fn cut_line(&mut self, row: usize) -> String {
        self.revision += 1;
        if self.lines.len() == 1 {
            return std::mem::take(&mut self.lines[0]);
        }
        self.lines.remove(row)
    }

    /// Insert whole lines above `row`.
    pub fn paste_lines(&mut self, row: usize, lines: &[String]) {
        for (i, l) in lines.iter().enumerate() {
            self.lines.insert(row + i, l.clone());
        }
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Buffer {
        Buffer::from_text(PathBuf::new(), "")
    }

    #[test]
    fn insert_and_backspace_wide_chars() {
        let mut b = empty();
        let (r, c) = b.insert_str(0, 0, "안녕 世界");
        assert_eq!((r, c), (0, 5));
        let (r, c) = b.backspace(r, c);
        assert_eq!(b.lines[0], "안녕 世");
        assert_eq!((r, c), (0, 4));
    }

    #[test]
    fn multiline_paste_splits_lines() {
        let mut b = empty();
        b.insert_str(0, 0, "ab");
        let (r, c) = b.insert_str(0, 1, "1\r\n한\n日本");
        assert_eq!(b.lines, vec!["a1", "한", "日本b"]);
        assert_eq!((r, c), (2, 2));
    }

    #[test]
    fn backspace_joins_lines() {
        let mut b = empty();
        b.insert_str(0, 0, "가\n나");
        let (r, c) = b.backspace(1, 0);
        assert_eq!(b.lines, vec!["가나"]);
        assert_eq!((r, c), (0, 1));
    }

    #[test]
    fn range_extract_and_delete() {
        let mut b = Buffer::from_text(PathBuf::new(), "한글abc\n日本語\nxyz");
        assert_eq!(b.text_range((0, 1), (2, 1)), "글abc\n日本語\nx");
        b.delete_range((0, 1), (2, 1));
        assert_eq!(b.lines, vec!["한yz"]);
    }

    #[test]
    fn move_lines_swaps_with_neighbor() {
        let mut b = Buffer::from_text(PathBuf::new(), "a\nb\nc\nd");
        assert!(b.move_lines(1, 2, true));
        assert_eq!(b.lines, vec!["b", "c", "a", "d"]);
        assert!(b.move_lines(0, 1, false));
        assert_eq!(b.lines, vec!["a", "b", "c", "d"]);
        assert!(!b.move_lines(0, 0, true));
        assert!(!b.move_lines(3, 3, false));
    }
}
