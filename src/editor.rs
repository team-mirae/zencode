//! Editor state, key handling, and rendering. Keybindings follow
//! KEYBINDS.md.
//!
//! Screen layout (top to bottom):
//!   1 row    title bar (gradient)
//!   n rows   text area with line-number gutter
//!   1 row    status message / prompt
//!   1+ rows  shortcut bar, wrapped onto as many rows as the width needs
//!
//! Interface text avoids "ambiguous width" glyphs such as `·` or `│`, which
//! render as two columns in many CJK terminal setups. The arrows in the
//! shortcut bar are the exception; they are measured with the same width
//! rules as the text, so `ZENCODE_AMBIGUOUS_WIDE=1` keeps the layout right.

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use crossterm::queue;
use crossterm::style::{
    Attribute, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{self, BeginSynchronizedUpdate, EndSynchronizedUpdate};
use unicode_segmentation::UnicodeSegmentation;

use crate::brand;
use crate::buffer::{Buffer, Pos};
use crate::history::{EditKind, History, Snapshot};
use crate::letterblock;
use crate::search::{Match, Matcher};
use crate::text::{
    display_col, gidx_for_col, grapheme_count, grapheme_width, is_control, str_width, truncate,
    width_at,
};
use crate::theme::{ColorDepth, Rgb, Theme};

const STATUS_TTL: Duration = Duration::from_secs(4);
/// Clicks on the same cell within this long count as a double / triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);
const WHEEL_LINES: isize = 3;

/// Always-visible shortcut bar, alternatives included. The bar fills
/// row by row; the order keeps the long entries in one column, so four
/// columns fit in 80.
///
/// Entries with an action can also be clicked. Those without are either
/// movement keys or not safe to trigger by a stray click (⌃⇧X discards
/// changes).
const SHORTCUTS: &[(&str, &str, Option<Action>)] = &[
    ("⌃X", "Exit", Some(Action::Exit)),
    ("⌃S", "Save", Some(Action::Save)),
    ("⌃O", "Export", Some(Action::Export)),
    ("⌃⇧X", "Force Exit", None),
    ("⌃F", "Find", Some(Action::Find)),
    ("⌃G", "Find Exact", Some(Action::FindExact)),
    ("⌃/ F1", "Help", Some(Action::Help)),
    ("⌃←→/⌥←→", "Letterblock", None),
    ("⌃C", "Copy", Some(Action::Copy)),
    ("⌃B", "Extract", Some(Action::Extract)),
    ("⌃V", "Paste", Some(Action::Paste)),
    ("⌃/⌥ ⌫⌦", "Delete Letterblock", None),
    ("⌃A", "Select All", Some(Action::SelectAll)),
    ("⇧←→↑↓", "Select", None),
    ("⌃E/⌥E", "Undo", Some(Action::Undo)),
    ("⌃↑↓/⌥↑↓", "Move Line", None),
    ("⌃R/⌥R", "Redo", Some(Action::Redo)),
];

/// Commands reachable from both a key and a click on the shortcut bar.
#[derive(Clone, Copy)]
enum Action {
    Exit,
    Save,
    Export,
    Find,
    FindExact,
    Help,
    Copy,
    Extract,
    Paste,
    SelectAll,
    Undo,
    Redo,
}

/// Shown below the brand header in the help box.
const HELP_LINES: &[&str] = &[
    "A nano-style 全角 text editor",
    "",
    "⌃X   Exit                ⌃⇧X  Exit without saving",
    "⌃S   Save                ⌃O   Export to a new file",
    "⌃F   Find                ⌃G   Find exactly",
    "Tab  Next match          ⇧Tab Previous match",
    "⌃/   Help  (or F1)",
    "⌃C   Copy                ⌃B   Extract (cut)",
    "⌃V   Paste               ⌃A   Select all",
    "⌃E   Undo  (or ⌥E)       ⌃R   Redo  (or ⌥R)",
    "",
    "⇧ + any movement         Extend the selection",
    "⌃←→  ⌥←→  ⌥B ⌥F          Move by letterblock",
    "⌃⌫   ⌥⌫   ⌃W             Delete letterblock left",
    "⌃⌦   ⌥⌦   ⌥D             Delete letterblock right",
    "⌃↑↓  ⌥↑↓                 Move line up / down",
    "Home End PgUp PgDn       Line start / end, page",
    "⌃Home  ⌃End              Top / bottom of file",
    "Esc                      Clear the selection",
    "",
    "With nothing selected, ⌃C and ⌃B take the whole line.",
    "Letterblocks: words; 日本語 splits where the script",
    "changes; 한국어 splits off 조사 and 어미 (학교|에서는).",
    "Find ignores case, matches 초성 (ㅎㄱ finds 한국),",
    "and turns かな into kanji (しんじゅく finds 新宿).",
];

enum PromptKind {
    Export,
    ConfirmOverwrite(PathBuf),
    Search { exact: bool },
    ConfirmQuit,
}

struct Prompt {
    kind: PromptKind,
    input: String,
    /// The input was pre-filled; the first edit replaces it instead of
    /// appending to it.
    fresh: bool,
}

impl Prompt {
    fn label(&self) -> &'static str {
        match self.kind {
            PromptKind::Export => "Export to: ",
            PromptKind::ConfirmOverwrite(_) => "File exists. Overwrite?  (Y)es  (N)o ",
            PromptKind::Search { exact: false } => "Find: ",
            PromptKind::Search { exact: true } => "Find exactly: ",
            PromptKind::ConfirmQuit => "Save modified buffer?  (Y)es  (N)o  (⌃C) Cancel ",
        }
    }

    fn backspace(&mut self) {
        if std::mem::take(&mut self.fresh) {
            self.input.clear();
        } else {
            self.input.pop();
        }
    }

    fn is_yes_no(&self) -> bool {
        matches!(
            self.kind,
            PromptKind::ConfirmOverwrite(_) | PromptKind::ConfirmQuit
        )
    }
}

/// A search in progress: matches update live as the query is typed.
struct SearchState {
    exact: bool,
    /// Cursor and selection when the search began, restored on cancel.
    origin: (Pos, Option<Pos>),
    matches: Vec<Match>,
    current: Option<usize>,
    /// Kanji spellings tried for a kana query.
    candidates: Vec<String>,
}

/// The last left click, for double / triple click and dragging.
struct Click {
    at: Instant,
    cell: (u16, u16),
    count: u8,
    /// Where the press landed, as the anchor for a drag.
    origin: Pos,
}

struct Status {
    text: String,
    error: bool,
    at: Instant,
}

/// zencode's own clipboard. A copy or extract made with nothing selected
/// takes the whole line, and pasting it inserts a whole line again.
struct Clip {
    text: String,
    linewise: bool,
}

#[derive(Clone, Copy)]
enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

pub struct Editor {
    buf: Buffer,
    theme: Theme,
    /// Keep clipboard contents inside zencode: no OSC 52 out, no pastes in.
    sandboxed: bool,
    /// Cursor row and grapheme index within that row.
    cy: usize,
    cx: usize,
    /// Display column the cursor tries to stay in when moving vertically.
    want_col: usize,
    /// Other end of the selection; the cursor is always one end.
    anchor: Option<Pos>,
    row_off: usize,
    col_off: usize,
    width: u16,
    height: u16,
    status: Option<Status>,
    clip: Option<Clip>,
    /// Text waiting to be sent to the terminal clipboard on the next draw.
    osc52: Option<String>,
    prompt: Option<Prompt>,
    show_help: bool,
    last_search: String,
    search: Option<SearchState>,
    history: History,
    /// Content state now and at the last save; they differ when modified.
    state: u64,
    saved_state: u64,
    last_click: Option<Click>,
    /// Clickable shortcut under the mouse pointer.
    hover: Option<usize>,
    /// Keep the cursor in view. Turned off while the wheel scrolls the view
    /// on its own, and back on with the next key press or click.
    follow_cursor: bool,
    quit: bool,
}

impl Editor {
    pub fn new(path: PathBuf, sandboxed: bool) -> Result<Self> {
        let (buf, created) = Buffer::open(&path)?;
        let (width, height) = terminal::size()?;
        let mut ed = Self {
            buf,
            theme: Theme::sakura(ColorDepth::detect()),
            sandboxed,
            cy: 0,
            cx: 0,
            want_col: 0,
            anchor: None,
            row_off: 0,
            col_off: 0,
            width,
            height,
            status: None,
            clip: None,
            osc52: None,
            prompt: None,
            show_help: false,
            last_search: String::new(),
            search: None,
            history: History::default(),
            state: 0,
            saved_state: 0,
            last_click: None,
            hover: None,
            follow_cursor: true,
            quit: false,
        };
        if created {
            ed.set_status(format!("Created {}", path.display()));
        } else {
            let n = ed.buf.lines.len();
            ed.set_status(format!("Read {n} line{}", plural(n)));
        }
        Ok(ed)
    }

    pub fn run(&mut self, out: &mut impl Write) -> Result<()> {
        let mut redraw = true;
        while !self.quit {
            if redraw {
                self.draw(out)?;
            }
            // Poll with a timeout so expired status messages get cleared.
            if !event::poll(Duration::from_millis(500))? {
                redraw = self.status.is_some();
                continue;
            }
            redraw = match event::read()? {
                // Windows reports key releases too; only act on presses.
                Event::Key(k) if k.kind != KeyEventKind::Release => {
                    self.on_key(k);
                    true
                }
                Event::Paste(s) => {
                    self.on_paste(&s);
                    true
                }
                Event::Mouse(m) => self.on_mouse(m),
                Event::Resize(w, h) => {
                    self.width = w;
                    self.height = h;
                    true
                }
                _ => false,
            };
        }
        Ok(())
    }

    fn perform(&mut self, action: Action) {
        match action {
            Action::Exit => self.try_quit(),
            Action::Save => self.save(false),
            Action::Export => self.open_prompt(PromptKind::Export),
            Action::Find => self.open_search(false),
            Action::FindExact => self.open_search(true),
            Action::Help => self.show_help = true,
            Action::Copy => self.copy(),
            Action::Extract => self.extract(),
            Action::Paste => self.paste(),
            Action::SelectAll => self.select_all(),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
        }
    }

    // ----- input -------------------------------------------------------

    fn on_key(&mut self, k: KeyEvent) {
        self.follow_cursor = true;
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.prompt.is_some() {
            self.on_prompt_key(k);
            return;
        }

        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);

        match k.code {
            // ⌃⇧X is only distinguishable from ⌃X in terminals that speak
            // the kitty keyboard protocol; elsewhere it exits gracefully.
            KeyCode::Char(c) if ctrl && (c == 'X' || (c == 'x' && shift)) => self.quit = true,
            KeyCode::Char('x') if ctrl => self.perform(Action::Exit),
            KeyCode::Char('s') if ctrl => self.perform(Action::Save),
            KeyCode::Char('o') if ctrl => self.perform(Action::Export),
            KeyCode::Char('f') if ctrl => self.perform(Action::Find),
            KeyCode::Char('g') if ctrl => self.perform(Action::FindExact),
            // ⌃/ arrives as ^_ (0x1F) in most terminals, which crossterm
            // reports as ⌃7; with the kitty protocol it's a real ⌃/.
            KeyCode::Char('/' | '7' | '_') if ctrl => self.perform(Action::Help),
            KeyCode::F(1) => self.perform(Action::Help),
            KeyCode::Char('c') if ctrl => self.perform(Action::Copy),
            KeyCode::Char('b') if ctrl => self.perform(Action::Extract),
            KeyCode::Char('v') if ctrl => self.perform(Action::Paste),
            KeyCode::Char('a') if ctrl => self.perform(Action::SelectAll),
            KeyCode::Char('e') if ctrl || alt => self.perform(Action::Undo),
            KeyCode::Char('r') if ctrl || alt => self.perform(Action::Redo),
            // Many terminals send ⌃⌫ as ^H, and iTerm2's "natural text
            // editing" sends ⌥⌫ as ^W and ⌥⌦ as ESC d.
            KeyCode::Char('h' | 'w') if ctrl => self.delete_block(true),
            KeyCode::Char('d') if alt => self.delete_block(false),
            KeyCode::Char('d') if ctrl => self.delete(),
            KeyCode::Backspace if ctrl || alt => self.delete_block(true),
            KeyCode::Delete if ctrl || alt => self.delete_block(false),

            // Terminals that map ⌥ to Meta send ⌥← / ⌥→ as ESC b / ESC f.
            KeyCode::Char(c @ ('b' | 'B')) if alt => {
                self.motion(Motion::WordLeft, shift || c == 'B')
            }
            KeyCode::Char(c @ ('f' | 'F')) if alt => {
                self.motion(Motion::WordRight, shift || c == 'F')
            }
            // With ⇧ held the user is adjusting a selection, so ⇧⌃↑↓ and
            // ⇧⌥↑↓ fall through to plain ⇧↑↓ instead of moving lines.
            KeyCode::Up if (ctrl || alt) && !shift => self.move_lines(true),
            KeyCode::Down if (ctrl || alt) && !shift => self.move_lines(false),
            KeyCode::Left if ctrl || alt => self.motion(Motion::WordLeft, shift),
            KeyCode::Right if ctrl || alt => self.motion(Motion::WordRight, shift),
            KeyCode::Home if ctrl => self.motion(Motion::Top, shift),
            KeyCode::End if ctrl => self.motion(Motion::Bottom, shift),

            KeyCode::Left => self.motion(Motion::Left, shift),
            KeyCode::Right => self.motion(Motion::Right, shift),
            KeyCode::Up => self.motion(Motion::Up, shift),
            KeyCode::Down => self.motion(Motion::Down, shift),
            KeyCode::Home => self.motion(Motion::Home, shift),
            KeyCode::End => self.motion(Motion::End, shift),
            KeyCode::PageUp => self.motion(Motion::PageUp, shift),
            KeyCode::PageDown => self.motion(Motion::PageDown, shift),

            KeyCode::Esc => {
                self.anchor = None;
                self.history.break_step();
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Enter => self.edit(EditKind::Other, |e| e.insert("\n")),
            KeyCode::Tab => self.edit(EditKind::Type, |e| e.insert("\t")),
            KeyCode::Char(c) if !ctrl && !alt => {
                self.edit(EditKind::Type, |e| e.insert(c.encode_utf8(&mut [0; 4])))
            }
            _ => {}
        }
    }

    /// Click to place the cursor (⇧ to extend the selection), drag to
    /// select, double-click to select a letterblock, triple-click to
    /// select the line, and scroll with the wheel.
    /// Returns whether the screen needs redrawing; plain pointer movement
    /// only does when it changes which shortcut is hovered.
    fn on_mouse(&mut self, m: MouseEvent) -> bool {
        if let MouseEventKind::Moved = m.kind {
            let hover = self
                .shortcut_at(m.column, m.row)
                .filter(|_| self.prompt.is_none());
            return std::mem::replace(&mut self.hover, hover) != hover;
        }
        if self.show_help {
            if let MouseEventKind::Down(_) = m.kind {
                self.show_help = false;
            }
            return true;
        }
        if self.prompt.is_some() {
            return false;
        }
        if let MouseEventKind::Down(MouseButton::Left) = m.kind
            && let Some(i) = self.shortcut_at(m.column, m.row)
        {
            self.follow_cursor = true;
            if let Some(action) = SHORTCUTS[i].2 {
                self.perform(action);
            }
            return true;
        }
        self.on_text_mouse(m);
        true
    }

    /// Index of the clickable shortcut under a screen cell.
    fn shortcut_at(&self, x: u16, y: u16) -> Option<usize> {
        let bar_y = 2 + self.text_rows();
        let (cols, rows) = self.shortcut_layout();
        let r = (y as usize).checked_sub(bar_y).filter(|&r| r < rows)?;
        let mut left = 0;
        for (c, &w) in cols.iter().enumerate() {
            if (left..left + w).contains(&(x as usize)) {
                let i = r * cols.len() + c;
                return SHORTCUTS.get(i).and_then(|s| s.2).map(|_| i);
            }
            left += w;
        }
        None
    }

    /// Mouse events over the text: click, drag, multi-click and wheel.
    fn on_text_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollUp => self.scroll_view(-WHEEL_LINES),
            MouseEventKind::ScrollDown => self.scroll_view(WHEEL_LINES),
            MouseEventKind::Down(MouseButton::Left) => {
                let Some((row, col)) = self.cell_to_text(m.column, m.row, false) else {
                    return;
                };
                let cell = (m.column, m.row);
                let count = match &self.last_click {
                    Some(c) if c.cell == cell && c.at.elapsed() < MULTI_CLICK => c.count % 3 + 1,
                    _ => 1,
                };
                self.follow_cursor = true;
                self.history.break_step();
                let line = &self.buf.lines[row];
                let origin = (row, nearest_boundary(line, col));
                match count {
                    1 if m.modifiers.contains(KeyModifiers::SHIFT) => {
                        self.anchor.get_or_insert((self.cy, self.cx));
                        self.move_to(origin);
                    }
                    1 => {
                        self.anchor = None;
                        self.move_to(origin);
                    }
                    2 => {
                        let block = letterblock::block_at(line, gidx_for_col(line, col));
                        self.anchor = Some((row, block.start));
                        self.move_to((row, block.end));
                    }
                    _ => {
                        let len = self.buf.line_len(row);
                        self.anchor = Some((row, 0));
                        self.move_to((row, len));
                    }
                }
                self.last_click = Some(Click {
                    at: Instant::now(),
                    cell,
                    count,
                    origin,
                });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(origin) = self.last_click.as_ref().map(|c| c.origin) else {
                    return;
                };
                let Some((row, col)) = self.cell_to_text(m.column, m.row, true) else {
                    return;
                };
                self.follow_cursor = true;
                self.anchor.get_or_insert(origin);
                let gidx = nearest_boundary(&self.buf.lines[row], col);
                self.move_to((row, gidx));
            }
            _ => {}
        }
    }

    /// Map a screen cell to (buffer row, display column). Outside the text
    /// area this gives `None`, unless `clamp` pins it to the nearest row,
    /// as a drag past the edge should.
    fn cell_to_text(&self, x: u16, y: u16, clamp: bool) -> Option<(usize, usize)> {
        let rows = self.text_rows();
        if rows == 0 {
            return None;
        }
        let y = y as usize;
        let screen_row = if (1..=rows).contains(&y) {
            y - 1
        } else if clamp {
            y.clamp(1, rows) - 1
        } else {
            return None;
        };
        let row = (self.row_off + screen_row).min(self.buf.lines.len() - 1);
        let col = (x as usize).saturating_sub(self.gutter_width()) + self.col_off;
        Some((row, col))
    }

    fn scroll_view(&mut self, delta: isize) {
        let last = self.buf.lines.len().saturating_sub(1);
        self.row_off = self.row_off.saturating_add_signed(delta).min(last);
        self.follow_cursor = false;
    }

    /// Bracketed paste: text coming from the system clipboard via the
    /// terminal (⌘V, ⌃⇧V, middle click).
    fn on_paste(&mut self, s: &str) {
        if self.sandboxed {
            self.set_error("Sandboxed: pasting from the system clipboard is blocked");
            return;
        }
        if let Some(p) = &mut self.prompt {
            if !p.is_yes_no() {
                if std::mem::take(&mut p.fresh) {
                    p.input.clear();
                }
                p.input.push_str(s.lines().next().unwrap_or(""));
                self.refresh_search();
            }
            return;
        }
        self.edit(EditKind::Other, |e| e.insert(s));
    }

    fn on_prompt_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let cancel = k.code == KeyCode::Esc || (ctrl && k.code == KeyCode::Char('c'));
        let Some(p) = &mut self.prompt else { return };

        if cancel {
            self.prompt = None;
            if self.search.is_some() {
                self.restore_origin();
                self.search = None;
            }
            self.set_status("Cancelled");
            return;
        }

        if p.is_yes_no() {
            let yes = match k.code {
                KeyCode::Char('y' | 'Y') => true,
                KeyCode::Char('n' | 'N') => false,
                _ => return,
            };
            let p = self.prompt.take().unwrap();
            match p.kind {
                PromptKind::ConfirmQuit if yes => self.save(true),
                PromptKind::ConfirmQuit => self.quit = true,
                PromptKind::ConfirmOverwrite(path) if yes => self.export_to(path),
                _ => self.set_status("Cancelled"),
            }
            return;
        }

        match k.code {
            KeyCode::Enter => {
                let p = self.prompt.take().unwrap();
                self.submit_prompt(p);
            }
            KeyCode::Tab if k.modifiers.contains(KeyModifiers::SHIFT) => self.step_search(false),
            KeyCode::Tab => self.step_search(true),
            KeyCode::BackTab => self.step_search(false),
            KeyCode::Backspace => {
                p.backspace();
                self.refresh_search();
            }
            KeyCode::Char('h') if ctrl => {
                p.backspace();
                self.refresh_search();
            }
            KeyCode::Char(c) if !ctrl => {
                if std::mem::take(&mut p.fresh) {
                    p.input.clear();
                }
                p.input.push(c);
                self.refresh_search();
            }
            _ => {}
        }
    }

    fn open_prompt(&mut self, kind: PromptKind) {
        let input = match kind {
            PromptKind::Search { .. } => self.last_search.clone(),
            _ => String::new(),
        };
        self.status = None;
        let fresh = !input.is_empty();
        self.prompt = Some(Prompt { kind, input, fresh });
    }

    fn submit_prompt(&mut self, p: Prompt) {
        match p.kind {
            PromptKind::Export => {
                if p.input.is_empty() {
                    self.set_status("Cancelled");
                    return;
                }
                let path = PathBuf::from(p.input);
                if path.exists() {
                    self.open_prompt(PromptKind::ConfirmOverwrite(path));
                } else {
                    self.export_to(path);
                }
            }
            PromptKind::Search { .. } => {
                let Some(s) = self.search.take() else { return };
                if p.input.is_empty() {
                    self.set_status("Cancelled");
                    return;
                }
                // The current match stays selected.
                match s.current {
                    Some(i) => self.set_status(format!("Match {} of {}", i + 1, s.matches.len())),
                    None => self.set_status(format!("\"{}\" not found", p.input)),
                }
                self.last_search = p.input;
            }
            PromptKind::ConfirmOverwrite(_) | PromptKind::ConfirmQuit => {}
        }
    }

    // ----- commands ----------------------------------------------------

    fn dirty(&self) -> bool {
        self.state != self.saved_state
    }

    fn try_quit(&mut self) {
        if self.dirty() {
            self.open_prompt(PromptKind::ConfirmQuit);
        } else {
            self.quit = true;
        }
    }

    fn save(&mut self, then_quit: bool) {
        match self.buf.save() {
            Ok(n) => {
                self.saved_state = self.state;
                self.history.break_step();
                self.set_status(format!("Wrote {n} line{}", plural(n)));
                if then_quit {
                    self.quit = true;
                }
            }
            Err(e) => self.set_error(format!("Error writing: {e:#}")),
        }
    }

    /// Write a copy to `path`; the buffer stays bound to its own file.
    fn export_to(&mut self, path: PathBuf) {
        match self.buf.write_to(&path) {
            Ok(n) => self.set_status(format!(
                "Exported {n} line{} to {}",
                plural(n),
                path.display()
            )),
            Err(e) => self.set_error(format!("Error writing: {e:#}")),
        }
    }

    fn insert(&mut self, s: &str) {
        self.delete_selection();
        (self.cy, self.cx) = self.buf.insert_str(self.cy, self.cx, s);
        self.sync_want_col();
    }

    fn backspace(&mut self) {
        self.edit(EditKind::Backspace, |e| {
            if !e.delete_selection() {
                (e.cy, e.cx) = e.buf.backspace(e.cy, e.cx);
                e.sync_want_col();
            }
        });
    }

    fn delete(&mut self) {
        self.edit(EditKind::Delete, |e| {
            if !e.delete_selection() {
                e.buf.delete(e.cy, e.cx);
            }
        });
    }

    /// Delete to the edge of the letterblock on one side of the cursor. At
    /// the start or end of a line this joins lines, like ⌫ / ⌦.
    fn delete_block(&mut self, left: bool) {
        let kind = if left {
            EditKind::Backspace
        } else {
            EditKind::Delete
        };
        self.edit(kind, |e| {
            if e.delete_selection() {
                return;
            }
            let (cy, cx) = (e.cy, e.cx);
            let line = &e.buf.lines[cy];
            if left && cx == 0 {
                (e.cy, e.cx) = e.buf.backspace(cy, cx);
                e.sync_want_col();
            } else if left {
                let to = letterblock::left(line, cx);
                e.buf.delete_range((cy, to), (cy, cx));
                e.move_to((cy, to));
            } else if cx == e.buf.line_len(cy) {
                e.buf.delete(cy, cx);
            } else {
                let to = letterblock::right(line, cx);
                e.buf.delete_range((cy, cx), (cy, to));
            }
        });
    }

    // ----- undo & redo --------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.buf.lines.clone(),
            cursor: (self.cy, self.cx),
            anchor: self.anchor,
            state: self.state,
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.buf.lines = s.lines;
        self.buf.revision += 1;
        self.state = s.state;
        self.anchor = s.anchor;
        self.move_to(s.cursor);
    }

    /// Run a buffer edit and record it for undo. The snapshot is only
    /// taken when the edit starts a new step, and dropped if nothing
    /// changed.
    fn edit(&mut self, kind: EditKind, f: impl FnOnce(&mut Self)) {
        // Typing over a selection replaces it, which is its own step.
        // Moving a selected block of lines repeatedly is still one step.
        let selection_ok = kind == EditKind::MoveLines || self.selection().is_none();
        let joins = selection_ok && self.history.continues(kind, (self.cy, self.cx));
        let before = (!joins).then(|| self.snapshot());
        let revision = self.buf.revision;
        f(self);
        if self.buf.revision == revision {
            return;
        }
        self.state = self.buf.revision;
        self.history.record(before, kind, (self.cy, self.cx));
    }

    fn undo(&mut self) {
        match self.history.undo(self.snapshot()) {
            Some(prev) => self.restore(prev),
            None => self.set_status("Nothing to undo"),
        }
    }

    fn redo(&mut self) {
        match self.history.redo(self.snapshot()) {
            Some(next) => self.restore(next),
            None => self.set_status("Nothing to redo"),
        }
    }

    // ----- selection & clipboard ----------------------------------------

    /// The selected range, ordered, or `None` if nothing is selected.
    fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        let c = (self.cy, self.cx);
        match a.cmp(&c) {
            std::cmp::Ordering::Less => Some((a, c)),
            std::cmp::Ordering::Greater => Some((c, a)),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// Delete the selection, if any, and clear it. Returns whether
    /// anything was deleted.
    fn delete_selection(&mut self) -> bool {
        let sel = self.selection();
        self.anchor = None;
        let Some((start, end)) = sel else {
            return false;
        };
        self.buf.delete_range(start, end);
        self.move_to(start);
        true
    }

    fn select_all(&mut self) {
        self.history.break_step();
        self.anchor = Some((0, 0));
        self.move_to(self.buf.end());
    }

    fn set_clip(&mut self, text: String, linewise: bool) {
        if !self.sandboxed {
            let mut system = text.clone();
            if linewise {
                system.push('\n');
            }
            self.osc52 = Some(system);
        }
        self.clip = Some(Clip { text, linewise });
    }

    fn clip_status(&mut self, verb: &str, what: String) {
        let suffix = if self.sandboxed { " (sandboxed)" } else { "" };
        self.set_status(format!("{verb} {what}{suffix}"));
    }

    fn copy(&mut self) {
        if let Some((start, end)) = self.selection() {
            let text = self.buf.text_range(start, end);
            let what = describe(&text);
            self.set_clip(text, false);
            self.clip_status("Copied", what);
        } else {
            let line = self.buf.lines[self.cy].clone();
            self.set_clip(line, true);
            self.clip_status("Copied", "line".into());
        }
    }

    fn extract(&mut self) {
        self.edit(EditKind::Other, Self::extract_inner);
    }

    fn extract_inner(&mut self) {
        if let Some((start, end)) = self.selection() {
            let text = self.buf.text_range(start, end);
            let what = describe(&text);
            self.set_clip(text, false);
            self.delete_selection();
            self.clip_status("Extracted", what);
        } else {
            let line = self.buf.cut_line(self.cy);
            self.set_clip(line, true);
            self.move_to((self.cy.min(self.buf.lines.len() - 1), 0));
            self.clip_status("Extracted", "line".into());
        }
    }

    fn paste(&mut self) {
        self.edit(EditKind::Other, Self::paste_inner);
    }

    fn paste_inner(&mut self) {
        let Some(clip) = &self.clip else {
            self.set_status("Clipboard is empty");
            return;
        };
        let text = clip.text.clone();
        if clip.linewise && self.selection().is_none() {
            // Whole lines go in above the cursor line, like they came out.
            self.anchor = None;
            self.buf.paste_lines(self.cy, &[text]);
            self.cy += 1;
            self.cx = gidx_for_col(&self.buf.lines[self.cy], self.want_col);
        } else {
            self.insert(&text);
        }
    }

    // ----- search --------------------------------------------------------

    fn open_search(&mut self, exact: bool) {
        self.history.break_step();
        self.search = Some(SearchState {
            exact,
            origin: ((self.cy, self.cx), self.anchor),
            matches: Vec::new(),
            current: None,
            candidates: Vec::new(),
        });
        self.open_prompt(PromptKind::Search { exact });
        self.refresh_search();
    }

    /// Re-run the search for the current query and select the first match
    /// at or after where the search began.
    fn refresh_search(&mut self) {
        let (Some(p), Some(s)) = (&self.prompt, &mut self.search) else {
            return;
        };
        if p.input.is_empty() {
            s.matches.clear();
            s.candidates.clear();
            s.current = None;
        } else {
            let matcher = Matcher::new(&p.input, s.exact);
            s.matches = matcher.find_all(&self.buf.lines);
            s.candidates = matcher.candidates;
            let origin = s.origin.0;
            s.current = (!s.matches.is_empty()).then(|| {
                s.matches
                    .iter()
                    .position(|m| (m.row, m.start) >= origin)
                    .unwrap_or(0)
            });
        }
        self.show_current_match();
    }

    /// Tab / ⇧Tab: move to the next or previous match, wrapping around.
    fn step_search(&mut self, forward: bool) {
        let Some(s) = &mut self.search else { return };
        let (Some(i), n) = (s.current, s.matches.len()) else {
            return;
        };
        s.current = Some(if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        });
        self.show_current_match();
    }

    /// Select the current match, or go back to the origin if there is none.
    fn show_current_match(&mut self) {
        let Some(s) = &self.search else { return };
        match s.current.map(|i| s.matches[i]) {
            Some(m) => {
                self.anchor = Some((m.row, m.start));
                self.move_to((m.row, m.end));
            }
            None => self.restore_origin(),
        }
    }

    fn restore_origin(&mut self) {
        let Some(s) = &self.search else { return };
        let (pos, anchor) = s.origin;
        self.anchor = anchor;
        self.move_to(pos);
    }

    /// Swap the cursor line (or every line the selection touches) with
    /// the line above or below; cursor and selection travel with it.
    fn move_lines(&mut self, up: bool) {
        self.edit(EditKind::MoveLines, |e| e.move_lines_inner(up));
    }

    fn move_lines_inner(&mut self, up: bool) {
        let (r0, r1) = match self.selection() {
            // A selection ending at column 0 doesn't include that line.
            Some((s, e)) if e.1 == 0 && e.0 > s.0 => (s.0, e.0 - 1),
            Some((s, e)) => (s.0, e.0),
            None => (self.cy, self.cy),
        };
        if !self.buf.move_lines(r0, r1, up) {
            return;
        }
        // A selection end at column 0 of the line below the block can be
        // pushed past the last line; it then becomes the end of the file.
        let end = self.buf.end();
        let shift = |(row, col): Pos| {
            let row = if up { row - 1 } else { row + 1 };
            if row > end.0 { end } else { (row, col) }
        };
        (self.cy, self.cx) = shift((self.cy, self.cx));
        self.anchor = self.anchor.map(shift);
    }

    // ----- movement ----------------------------------------------------

    fn sync_want_col(&mut self) {
        self.want_col = display_col(&self.buf.lines[self.cy], self.cx);
    }

    fn move_to(&mut self, (row, gidx): Pos) {
        self.cy = row;
        self.cx = gidx.min(self.buf.line_len(row));
        self.sync_want_col();
    }

    /// Move the cursor. With `extend` the selection grows; without it,
    /// any selection is dropped (←/→ collapse it to the matching end).
    fn motion(&mut self, m: Motion, extend: bool) {
        self.history.break_step();
        if extend {
            self.anchor.get_or_insert((self.cy, self.cx));
        } else if let Some((start, end)) = self.selection() {
            self.anchor = None;
            match m {
                Motion::Left => return self.move_to(start),
                Motion::Right => return self.move_to(end),
                _ => {}
            }
        } else {
            self.anchor = None;
        }

        let (cy, cx) = (self.cy, self.cx);
        let len = self.buf.line_len(cy);
        let last = self.buf.lines.len() - 1;
        let rows = self.text_rows().max(1) as isize;
        match m {
            Motion::Left if cx > 0 => self.move_to((cy, cx - 1)),
            Motion::Left if cy > 0 => self.move_to((cy - 1, self.buf.line_len(cy - 1))),
            Motion::Right if cx < len => self.move_to((cy, cx + 1)),
            Motion::Right if cy < last => self.move_to((cy + 1, 0)),
            Motion::WordLeft if cx > 0 => {
                self.move_to((cy, letterblock::left(&self.buf.lines[cy], cx)))
            }
            Motion::WordLeft if cy > 0 => self.move_to((cy - 1, self.buf.line_len(cy - 1))),
            Motion::WordRight if cx < len => {
                self.move_to((cy, letterblock::right(&self.buf.lines[cy], cx)))
            }
            Motion::WordRight if cy < last => self.move_to((cy + 1, 0)),
            Motion::Up => self.move_vert(-1),
            Motion::Down => self.move_vert(1),
            Motion::PageUp => self.move_vert(-rows),
            Motion::PageDown => self.move_vert(rows),
            Motion::Home => self.move_to((cy, 0)),
            Motion::End => self.move_to((cy, len)),
            Motion::Top => self.move_to((0, 0)),
            Motion::Bottom => self.move_to(self.buf.end()),
            _ => {}
        }
    }

    fn move_vert(&mut self, delta: isize) {
        let last = self.buf.lines.len() - 1;
        let row = self.cy.saturating_add_signed(delta).min(last);
        self.cy = row;
        self.cx = gidx_for_col(&self.buf.lines[row], self.want_col);
    }

    // ----- layout ------------------------------------------------------

    /// Shortcut bar geometry: the width of each column, and the number of
    /// rows. Columns are sized to their widest entry, using as many as fit,
    /// and any leftover width is shared out evenly.
    fn shortcut_layout(&self) -> (Vec<usize>, usize) {
        let width = self.width as usize;
        let items: Vec<usize> = SHORTCUTS
            .iter()
            .map(|(k, l, _)| str_width(k) + 1 + str_width(l) + 2)
            .collect();
        let mut cols = vec![width];
        for n in (1..=items.len()).rev() {
            let widths: Vec<usize> = (0..n)
                .map(|c| items.iter().skip(c).step_by(n).copied().max().unwrap_or(0))
                .collect();
            let total: usize = widths.iter().sum();
            if total <= width {
                let extra = (width - total) / n;
                cols = widths.into_iter().map(|w| w + extra).collect();
                break;
            }
        }
        // Never let the bar take more than half of what's left for text.
        let max_rows = (self.height as usize).saturating_sub(2) / 2;
        let rows = SHORTCUTS.len().div_ceil(cols.len()).min(max_rows);
        (cols, rows)
    }

    fn text_rows(&self) -> usize {
        let (_, bar) = self.shortcut_layout();
        (self.height as usize).saturating_sub(2 + bar)
    }

    fn gutter_width(&self) -> usize {
        self.buf.lines.len().to_string().len().max(3) + 2
    }

    fn text_width(&self) -> usize {
        (self.width as usize)
            .saturating_sub(self.gutter_width())
            .max(1)
    }

    fn scroll(&mut self) {
        if !self.follow_cursor {
            return;
        }
        let rows = self.text_rows().max(1);
        if self.cy < self.row_off {
            self.row_off = self.cy;
        } else if self.cy >= self.row_off + rows {
            self.row_off = self.cy + 1 - rows;
        }

        let line = &self.buf.lines[self.cy];
        let col = display_col(line, self.cx);
        let w = width_at(line, self.cx);
        let avail = self.text_width();
        if col < self.col_off {
            self.col_off = col;
        } else if col + w > self.col_off + avail {
            self.col_off = col + w - avail;
        }
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    fn set_error(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    // ----- rendering ---------------------------------------------------

    fn draw(&mut self, out: &mut impl Write) -> Result<()> {
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.at.elapsed() > STATUS_TTL)
        {
            self.status = None;
        }
        self.scroll();

        // OSC 52: ask the terminal to put this on the system clipboard.
        // Terminals without support ignore it.
        if let Some(text) = self.osc52.take() {
            queue!(out, Print(format!("\x1b]52;c;{}\x07", BASE64.encode(text))))?;
        }

        queue!(out, BeginSynchronizedUpdate, Hide)?;
        if self.height >= 1 {
            self.draw_title(out)?;
        }
        self.draw_text(out)?;

        let status_y = 1 + self.text_rows();
        let mut cursor = None;
        if status_y < self.height as usize {
            cursor = self.draw_status(out, status_y as u16)?;
            self.draw_shortcuts(out, status_y as u16 + 1)?;
        }
        if self.show_help {
            self.draw_help(out)?;
        }

        // The cursor may be off screen while the wheel scrolls the view.
        let cursor = cursor.or_else(|| {
            let col = display_col(&self.buf.lines[self.cy], self.cx).checked_sub(self.col_off)?;
            let row = self.cy.checked_sub(self.row_off)?;
            (row < self.text_rows() && col < self.text_width())
                .then(|| ((self.gutter_width() + col) as u16, (1 + row) as u16))
        });
        queue!(out, ResetColor)?;
        if let Some((x, y)) = cursor.filter(|_| !self.show_help) {
            queue!(out, MoveTo(x, y), Show)?;
        }
        queue!(out, EndSynchronizedUpdate)?;
        out.flush()?;
        Ok(())
    }

    fn colors(&self, out: &mut impl Write, fg: Rgb, bg: Rgb) -> Result<()> {
        queue!(
            out,
            SetForegroundColor(self.theme.color(fg)),
            SetBackgroundColor(self.theme.color(bg))
        )?;
        Ok(())
    }

    fn spaces(out: &mut impl Write, n: usize) -> Result<()> {
        if n > 0 {
            queue!(out, Print(" ".repeat(n)))?;
        }
        Ok(())
    }

    fn draw_title(&self, out: &mut impl Write) -> Result<()> {
        let t = &self.theme;
        let name = self.buf.path.display().to_string();
        let modified = if self.dirty() { "  [Modified]" } else { "" };
        let badge = if self.sandboxed { " SANDBOXED " } else { "" };
        let line_text = &self.buf.lines[self.cy];
        let right = format!(
            "Ln {}, Col {}   UTF-8  {}  {} ",
            self.cy + 1,
            display_col(line_text, self.cx) + 1,
            self.buf.line_ending.label(),
            t.depth.label()
        );
        let rw = str_width(&right);
        let width = self.width as usize;

        // Show all three names when they fit next to the file name,
        // otherwise fall back to the English one.
        let full = format!(" {}  {}  ", brand::full_name(), brand::VERSION);
        let short = format!(" {} {}  ", brand::NAME_EN, brand::VERSION);
        let tail = format!("{badge}   {name}{modified}");
        let prefix = if str_width(&full) + str_width(&tail) + rw < width {
            full
        } else {
            short
        };
        let badge_cols = str_width(&prefix)..str_width(&prefix) + str_width(badge);
        let left = format!("{prefix}{tail}");
        let (left, lw) = truncate(&left, width);
        let line = if lw + rw < width {
            format!("{left}{}{right}", " ".repeat(width - lw - rw))
        } else {
            format!("{left}{}", " ".repeat(width - lw))
        };

        queue!(out, MoveTo(0, 0), SetAttribute(Attribute::Bold))?;
        if t.mono() {
            queue!(out, SetAttribute(Attribute::Reverse), Print(&line))?;
        } else {
            // Paint each cell along a horizontal gradient.
            let mut col = 0;
            for g in line.graphemes(true) {
                if badge_cols.contains(&col) {
                    self.colors(out, t.badge_fg, t.badge_bg)?;
                } else {
                    let frac = col as f32 / width.max(2).saturating_sub(1) as f32;
                    self.colors(out, t.title_fg, t.title_from.lerp(t.title_to, frac))?;
                }
                queue!(out, Print(g))?;
                col += grapheme_width(g, col);
            }
        }
        queue!(out, SetAttribute(Attribute::Reset))?;
        Ok(())
    }

    fn draw_text(&self, out: &mut impl Write) -> Result<()> {
        let t = &self.theme;
        let gutter = self.gutter_width();
        let avail = self.text_width();

        for screen_row in 0..self.text_rows() {
            let row = self.row_off + screen_row;
            queue!(out, MoveTo(0, (1 + screen_row) as u16))?;

            let Some(line) = self.buf.lines.get(row) else {
                self.colors(out, t.gutter_fg, t.gutter_bg)?;
                Self::spaces(out, gutter)?;
                self.colors(out, t.fg, t.bg)?;
                Self::spaces(out, avail)?;
                continue;
            };

            let current = row == self.cy;
            let bg = if current { t.cur_line_bg } else { t.bg };
            let num_fg = if current {
                t.gutter_cur_fg
            } else {
                t.gutter_fg
            };
            self.colors(out, num_fg, t.gutter_bg)?;
            if current {
                queue!(out, SetAttribute(Attribute::Bold))?;
            }
            queue!(out, Print(format!(" {:>w$} ", row + 1, w = gutter - 2)))?;
            queue!(out, SetAttribute(Attribute::NormalIntensity))?;

            self.draw_line(out, row, line, bg, avail)?;
        }
        Ok(())
    }

    /// Search matches on `row` (empty unless a search is in progress).
    fn row_matches(&self, row: usize) -> &[Match] {
        let Some(s) = &self.search else { return &[] };
        let from = s.matches.partition_point(|m| m.row < row);
        let to = s.matches.partition_point(|m| m.row <= row);
        &s.matches[from..to]
    }

    /// Selected grapheme range on `row`, plus whether the line break at
    /// its end is selected too.
    fn row_selection(&self, row: usize) -> Option<(std::ops::Range<usize>, bool)> {
        let ((r0, c0), (r1, c1)) = self.selection()?;
        if row < r0 || row > r1 {
            return None;
        }
        let start = if row == r0 { c0 } else { 0 };
        let end = if row == r1 { c1 } else { usize::MAX };
        Some((start..end, row < r1))
    }

    /// Draw one buffer line clipped to the horizontal viewport. A wide
    /// character cut by either edge is replaced by an edge marker so the
    /// columns after it stay aligned.
    fn draw_line(
        &self,
        out: &mut impl Write,
        row: usize,
        line: &str,
        bg: Rgb,
        avail: usize,
    ) -> Result<()> {
        let t = &self.theme;
        let sel = self.row_selection(row);
        let matches = self.row_matches(row);
        let mut col = 0;
        let mut printed = 0;
        let mut clipped = false;

        for (gi, g) in line.graphemes(true).enumerate() {
            let start = col;
            let w = grapheme_width(g, col);
            col += w;
            if col <= self.col_off {
                continue;
            }
            if start < self.col_off {
                let n = col - self.col_off;
                self.colors(out, t.edge_fg, bg)?;
                queue!(out, Print("<".repeat(n)))?;
                printed += n;
                continue;
            }
            if col - self.col_off > avail {
                let n = avail - printed;
                self.colors(out, t.edge_fg, bg)?;
                queue!(out, Print(">".repeat(n)))?;
                printed = avail;
                clipped = true;
                break;
            }

            let selected = sel.as_ref().is_some_and(|(r, _)| r.contains(&gi));
            let matched = !selected && matches.iter().any(|m| (m.start..m.end).contains(&gi));
            let cell_bg = if selected {
                t.select_bg
            } else if matched {
                t.match_bg
            } else {
                bg
            };
            if g == "\t" {
                self.colors(out, t.tab_fg, cell_bg)?;
                Self::spaces(out, w)?;
            } else if g == "\u{3000}" {
                let space_bg = if selected {
                    t.select_bg
                } else {
                    t.zenkaku_space_bg
                };
                self.colors(out, t.fg, space_bg)?;
                Self::spaces(out, w)?;
            } else if is_control(g) {
                self.colors(out, t.control_fg, cell_bg)?;
                queue!(out, Print("?"))?;
            } else {
                self.colors(out, if matched { t.match_fg } else { t.fg }, cell_bg)?;
                queue!(out, Print(g))?;
            }
            printed += w;
        }

        // One highlighted cell past the end marks a selected line break.
        let newline_selected = sel.is_some_and(|(_, nl)| nl);
        if newline_selected && !clipped && col >= self.col_off && printed < avail {
            self.colors(out, t.fg, t.select_bg)?;
            Self::spaces(out, 1)?;
            printed += 1;
        }

        self.colors(out, t.fg, bg)?;
        Self::spaces(out, avail.saturating_sub(printed))?;
        Ok(())
    }

    /// Returns the cursor position when a prompt is active.
    fn draw_status(&self, out: &mut impl Write, y: u16) -> Result<Option<(u16, u16)>> {
        let t = &self.theme;
        let width = self.width as usize;
        queue!(out, MoveTo(0, y))?;

        if let Some(p) = &self.prompt {
            let (label, lw) = truncate(p.label(), width);
            self.colors(out, t.prompt_fg, t.bar_bg)?;
            queue!(out, SetAttribute(Attribute::Bold), Print(label))?;
            queue!(out, SetAttribute(Attribute::NormalIntensity))?;

            // Show the tail of long input so the cursor stays on screen.
            let room = width.saturating_sub(lw + 1);
            let mut input = p.input.as_str();
            while str_width(input) > room {
                let next = input
                    .grapheme_indices(true)
                    .nth(1)
                    .map_or(input.len(), |(i, _)| i);
                input = &input[next..];
            }
            let iw = str_width(input);
            let input_bg = if p.fresh { t.select_bg } else { t.bar_bg };
            self.colors(out, t.fg, input_bg)?;
            queue!(out, Print(input))?;

            // Search info on the right: kanji candidates and match count.
            let info = self.search_info(&p.input);
            let infow = str_width(&info);
            if lw + iw + infow + 2 <= width {
                Self::spaces(out, width - lw - iw - infow)?;
                self.colors(out, t.label_fg, t.bar_bg)?;
                queue!(out, Print(&info))?;
            } else {
                Self::spaces(out, width.saturating_sub(lw + iw))?;
            }
            return Ok(Some(((lw + iw).min(width.saturating_sub(1)) as u16, y)));
        }

        self.colors(out, t.fg, t.bar_bg)?;
        if let Some(s) = &self.status {
            let msg = format!("[ {} ]", s.text);
            let (msg, mw) = truncate(&msg, width);
            let pad = (width - mw) / 2;
            Self::spaces(out, pad)?;
            self.colors(
                out,
                if s.error { t.error_fg } else { t.status_fg },
                t.bar_bg,
            )?;
            if t.mono() {
                queue!(out, SetAttribute(Attribute::Reverse))?;
            }
            queue!(out, SetAttribute(Attribute::Bold), Print(msg))?;
            queue!(out, SetAttribute(Attribute::Reset))?;
            self.colors(out, t.fg, t.bar_bg)?;
            Self::spaces(out, width - pad - mw)?;
        } else {
            Self::spaces(out, width)?;
        }
        Ok(None)
    }

    fn search_info(&self, query: &str) -> String {
        let Some(s) = &self.search else {
            return String::new();
        };
        if query.is_empty() {
            return String::new();
        }
        let mut info = String::new();
        if !s.candidates.is_empty() {
            let shown: Vec<&str> = s.candidates.iter().take(4).map(String::as_str).collect();
            info.push_str(&format!("= {}   ", shown.join(" ")));
        }
        match s.current {
            Some(i) => info.push_str(&format!("{}/{}   Tab ⇧Tab ", i + 1, s.matches.len())),
            None => info.push_str("no matches "),
        }
        info
    }

    fn draw_shortcuts(&self, out: &mut impl Write, y: u16) -> Result<()> {
        let t = &self.theme;
        let width = self.width as usize;
        let (cols, rows) = self.shortcut_layout();

        for r in 0..rows {
            queue!(out, MoveTo(0, y + r as u16))?;
            let mut used = 0;
            let row = SHORTCUTS
                .iter()
                .enumerate()
                .skip(r * cols.len())
                .take(cols.len());
            for ((i, (key, label, _)), &cell) in row.zip(&cols) {
                let (key, kw) = truncate(key, cell);
                self.colors(out, t.key_fg, t.key_bg)?;
                if t.mono() {
                    queue!(out, SetAttribute(Attribute::Reverse))?;
                }
                queue!(out, SetAttribute(Attribute::Bold), Print(key))?;
                queue!(out, SetAttribute(Attribute::Reset))?;

                let rest = cell - kw;
                let label = format!(" {label}");
                let (label, lw) = truncate(&label, rest);
                if self.hover == Some(i) {
                    // Highlight the label, but not the gap after it.
                    self.colors(out, t.fg, t.select_bg)?;
                    if t.mono() {
                        queue!(out, SetAttribute(Attribute::Underlined))?;
                    }
                    queue!(
                        out,
                        Print(label),
                        Print(" "),
                        SetAttribute(Attribute::Reset)
                    )?;
                    self.colors(out, t.label_fg, t.bar_bg)?;
                    Self::spaces(out, (rest - lw).saturating_sub(1))?;
                } else {
                    self.colors(out, t.label_fg, t.bar_bg)?;
                    queue!(out, Print(label))?;
                    Self::spaces(out, rest - lw)?;
                }
                used += cell;
            }
            self.colors(out, t.label_fg, t.bar_bg)?;
            Self::spaces(out, width.saturating_sub(used))?;
        }
        Ok(())
    }

    fn draw_help(&self, out: &mut impl Write) -> Result<()> {
        let t = &self.theme;
        let header = format!("{}  {}", brand::full_name(), brand::VERSION);
        let mut lines: Vec<&str> = std::iter::once(header.as_str())
            .chain(HELP_LINES.iter().copied())
            .collect();
        if self.sandboxed {
            lines.push("Sandboxed: the clipboard never leaves zencode.");
        }
        lines.extend(["", "Press any key to close"]);

        let inner = lines.iter().map(|l| str_width(l)).max().unwrap_or(0);
        let box_w = (inner + 6).min(self.width as usize);
        let box_h = (lines.len() + 2).min(self.height as usize);
        let x0 = (self.width as usize - box_w) / 2;
        let y0 = (self.height as usize - box_h) / 2;

        for dy in 0..box_h {
            queue!(out, MoveTo(x0 as u16, (y0 + dy) as u16))?;
            let text = dy
                .checked_sub(1)
                .and_then(|i| lines.get(i))
                .copied()
                .unwrap_or("");
            let fg = if dy == 1 { t.gutter_cur_fg } else { t.fg };
            self.colors(out, fg, t.cur_line_bg)?;
            if t.mono() {
                queue!(out, SetAttribute(Attribute::Reverse))?;
            }
            Self::spaces(out, 3.min(box_w))?;
            let (text, w) = truncate(text, box_w.saturating_sub(3));
            if dy == 1 {
                queue!(out, SetAttribute(Attribute::Bold))?;
            }
            queue!(out, Print(text), SetAttribute(Attribute::NormalIntensity))?;
            Self::spaces(out, box_w.saturating_sub(3 + w))?;
            queue!(out, SetAttribute(Attribute::Reset))?;
        }
        Ok(())
    }
}

/// Grapheme boundary nearest to display column `col`: clicking the right
/// half of a full-width character puts the cursor after it.
fn nearest_boundary(line: &str, col: usize) -> usize {
    let gidx = gidx_for_col(line, col);
    let start = display_col(line, gidx);
    let w = width_at(line, gidx);
    if gidx < grapheme_count(line) && col - start >= w.div_ceil(2) && w > 1 {
        gidx + 1
    } else {
        gidx
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// "12 characters" or "3 lines" for status messages.
fn describe(text: &str) -> String {
    let lines = text.split('\n').count();
    if lines > 1 {
        format!("{lines} lines")
    } else {
        let n = grapheme_count(text);
        format!("{n} character{}", plural(n))
    }
}
