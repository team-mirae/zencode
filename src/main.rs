mod brand;
mod buffer;
mod editor;
mod history;
mod letterblock;
mod search;
mod text;
mod theme;

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use crossterm::cursor::Show;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::style::ResetColor;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};

use editor::Editor;

fn usage() -> String {
    format!(
        "\
{} {}
A nano-style terminal editor with first-class 全角 (한글 / 日本語) support

USAGE:
    zencode [OPTIONS] FILE

    FILE is created if it doesn't exist. Without FILE, zencode prints its
    version and exits.

OPTIONS:
    -s, --sandboxed    keep the clipboard inside zencode: never write to or
                       accept pastes from the system clipboard
    -h, --help         print this help
    -V, --version      print the version

ENVIRONMENT:
    ZENCODE_COLOR=truecolor|256|16|none   override color detection
    ZENCODE_AMBIGUOUS_WIDE=1              treat East Asian ambiguous-width chars as 2 columns
    NO_COLOR=1                            disable colors",
        brand::full_name(),
        brand::VERSION
    )
}

/// Whether we pushed kitty keyboard flags and so must pop them on exit.
static KEYBOARD_ENHANCED: AtomicBool = AtomicBool::new(false);

/// Puts the terminal into raw/alternate-screen mode and restores it on drop,
/// including when unwinding from a panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        // Not every console supports bracketed paste; it's fine without it.
        // Sandboxed mode relies on it to recognize and refuse pastes.
        let _ = execute!(io::stdout(), EnableBracketedPaste);
        // Clicks, drags and the wheel. The terminal's own mouse selection
        // then needs a modifier (⌥ on macOS, ⇧ on most Linux terminals).
        let _ = execute!(io::stdout(), EnableMouseCapture);
        // The kitty keyboard protocol lets ⌃⇧X arrive as something other
        // than plain ⌃X. Terminals without it just send ⌃X.
        if matches!(supports_keyboard_enhancement(), Ok(true))
            && execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )
            .is_ok()
        {
            KEYBOARD_ENHANCED.store(true, Ordering::SeqCst);
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    if KEYBOARD_ENHANCED.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(io::stdout(), DisableMouseCapture);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    let _ = execute!(io::stdout(), ResetColor, Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

fn main() -> Result<()> {
    let mut path = None;
    let mut sandboxed = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(());
            }
            "-V" | "--version" => {
                println!("{} {}", brand::full_name(), brand::VERSION);
                return Ok(());
            }
            "-s" | "--sandboxed" => sandboxed = true,
            a if a.starts_with('-') && a != "-" => {
                anyhow::bail!("unknown option {a}\n\n{}", usage())
            }
            _ if path.is_none() => path = Some(PathBuf::from(arg)),
            _ => anyhow::bail!("only one file can be opened at a time\n\n{}", usage()),
        }
    }
    // zencode never invents a file name: no file means "just tell me
    // what you are".
    let Some(path) = path else {
        println!("{} {}", brand::full_name(), brand::VERSION);
        return Ok(());
    };

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    let mut editor = Editor::new(path, sandboxed)?;
    let _guard = TerminalGuard::enter()?;
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
    editor.run(&mut out)?;
    out.flush()?;
    Ok(())
}
