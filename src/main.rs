mod brand;
mod buffer;
mod cli;
mod editor;
mod history;
mod letterblock;
mod search;
mod text;
mod theme;

use std::io::{self, BufWriter, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::style::{Print, ResetColor};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};

use cli::{Command, Source};
use editor::Editor;

fn usage() -> String {
    format!(
        "\
{} {}
A nano-style terminal editor with first-class 全角 (한글 / 日本語) support

USAGE:
    zencode [OPTIONS] [[+LINE[,COLUMN]] FILE]...

    FILE is created if it doesn't exist; `-` reads the text from standard
    input. Several files are edited one after another. Without FILE,
    zencode prints its version and exits.

    +LINE[,COLUMN]   start at LINE and COLUMN (negative counts from the end)
    +/TEXT, +?TEXT   start at the first / last match of TEXT; put c before
                     the / for a case-sensitive (exact) search
    FILE:LINE[:COL]  also understood, when FILE exists (always with -g / -@)

OPTIONS:
    -s, --sandboxed       keep the clipboard inside zencode: never write to
                          or accept pastes from the system clipboard
    -g, --goto            read FILE:LINE[:COL] as a position (also -@,
                          --colonparsing)
    -v, --view            view only: the buffer can't be changed
    -t, --saveonexit      save changes on exit without asking
    -T, --tabsize=N       tab stops every N columns (default 4)
    -E, --tabstospaces    Tab inserts spaces
    -i, --autoindent      new lines start with the previous line's indent
    -L, --nonewlines      don't add a line break at the end of the file
    -u, --unix            save with LF line endings
    -k, --cutfromcursor   ⌃B with no selection cuts to the end of the line
    -B, --backup          keep the original as FILE~ on the first save
    -C, --backupdir=DIR   keep that backup in DIR instead (implies -B)
    -n, --noread          start empty and only write the file (for pipes)
    -R, --restricted      no export and no backups
        --nosoftwrap      start with word wrap off (⌃L toggles it)
    -x, --nohelp          hide the shortcut bar
        --nomouse         leave the mouse to the terminal
    -h, --help            print this help
    -V, --version         print the version

    Other nano options are accepted and ignored, as are -w / --wait.

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
/// Whether we saved the terminal's DEC modes and so must restore them.
static MODES_SAVED: AtomicBool = AtomicBool::new(false);

/// DEC private modes zencode changes but a parent program may rely on:
/// bracketed paste (2004) and cursor visibility (25). A program that
/// hands zencode its terminal, like Claude Code, can have them on and not
/// turn them back on afterwards. XTSAVE / XTRESTORE put back whatever was
/// there; terminals without them ignore both, and the modes end up off and
/// visible, as before.
const SAVE_MODES: &str = "\x1b[?2004s\x1b[?25s";
const RESTORE_MODES: &str = "\x1b[?2004r\x1b[?25r";

/// Whether zencode switched to the alternate screen itself and so must
/// switch back on exit.
static ENTERED_ALTERNATE: AtomicBool = AtomicBool::new(false);
const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";

/// Asks the terminal whether the alternate screen (mode 1049) is already
/// on: DECRQM, followed by a primary device attributes request that every
/// terminal answers, so one without DECRQM costs a round trip rather than
/// the whole timeout. Expects raw mode. No answer, or no tty: false.
#[cfg(unix)]
fn already_on_alternate_screen() -> bool {
    use std::time::{Duration, Instant};
    if unsafe { libc::isatty(libc::STDIN_FILENO) } != 1 {
        return false;
    }
    if execute!(io::stdout(), Print("\x1b[?1049$p\x1b[c")).is_err() {
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut reply = Vec::new();
    while !da1_complete(&reply) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut fds = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd; read into a local buffer of its length.
        if unsafe { libc::poll(&mut fds, 1, left.as_millis() as libc::c_int) } <= 0 {
            break;
        }
        let mut buf = [0u8; 256];
        let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        reply.extend_from_slice(&buf[..n as usize]);
    }
    alternate_screen_reported(&reply)
}

#[cfg(not(unix))]
fn already_on_alternate_screen() -> bool {
    false
}

/// `reply` holds a complete primary device attributes answer (`ESC [ ? … c`).
#[cfg_attr(not(unix), allow(dead_code))]
fn da1_complete(reply: &[u8]) -> bool {
    (0..reply.len()).any(|i| {
        let Some(rest) = reply[i..].strip_prefix(b"\x1b[?") else {
            return false;
        };
        let params = rest
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b';')
            .count();
        rest.get(params) == Some(&b'c')
    })
}

/// The DECRQM answer `ESC [ ? 1049 ; Ps $ y` says set (1) or permanently set (3).
#[cfg_attr(not(unix), allow(dead_code))]
fn alternate_screen_reported(reply: &[u8]) -> bool {
    [b"\x1b[?1049;1$y", b"\x1b[?1049;3$y"]
        .iter()
        .any(|r| reply.windows(r.len()).any(|w| w == *r))
}

/// Puts the terminal into raw/alternate-screen mode and restores it on drop,
/// including when unwinding from a panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter(mouse: bool) -> Result<Self> {
        enable_raw_mode()?;
        if execute!(io::stdout(), Print(SAVE_MODES)).is_ok() {
            MODES_SAVED.store(true, Ordering::SeqCst);
        }
        if already_on_alternate_screen() {
            // Started from a full-screen program (Claude Code's ⌃G, for one)
            // that didn't hand over its screen. There is only one alternate
            // screen, so entering it again does nothing, and leaving it on
            // exit would drop that program onto the normal screen while it
            // still draws as if it were full-screen. Stay on its screen.
            execute!(io::stdout(), Print(CLEAR_SCREEN))?;
        } else {
            execute!(io::stdout(), EnterAlternateScreen)?;
            ENTERED_ALTERNATE.store(true, Ordering::SeqCst);
        }
        // Not every console supports bracketed paste; it's fine without it.
        // Sandboxed mode relies on it to recognize and refuse pastes.
        let _ = execute!(io::stdout(), EnableBracketedPaste);
        // Clicks, drags and the wheel. The terminal's own mouse selection
        // then needs a modifier (⌥ on macOS, ⇧ on most Linux terminals).
        if mouse {
            let _ = execute!(io::stdout(), EnableMouseCapture);
        }
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
    if ENTERED_ALTERNATE.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), ResetColor, Show, LeaveAlternateScreen);
    } else {
        // Nested: leave the parent's alternate screen blank for it to redraw.
        let _ = execute!(io::stdout(), ResetColor, Show, Print(CLEAR_SCREEN));
    }
    if MODES_SAVED.swap(false, Ordering::SeqCst) {
        let _ = execute!(io::stdout(), Print(RESTORE_MODES));
    }
    let _ = disable_raw_mode();
}

/// Put the terminal back on stdin once piped input has been read. This
/// opens the terminal's own device (like /dev/ttys003) rather than
/// /dev/tty, which crossterm would fall back to but macOS can't poll.
#[cfg(unix)]
fn reattach_stdin() -> io::Result<()> {
    use std::ffi::{CStr, OsStr};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    let device = [libc::STDOUT_FILENO, libc::STDERR_FILENO]
        .into_iter()
        .find_map(|fd| {
            // SAFETY: ttyname returns null or a C string that stays valid
            // until the next call; it's copied out right away.
            let name = unsafe { libc::ttyname(fd) };
            let name = (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) })?;
            Some(PathBuf::from(OsStr::from_bytes(name.to_bytes())))
        })
        .unwrap_or_else(|| PathBuf::from("/dev/tty"));
    let tty = std::fs::File::options()
        .read(true)
        .write(true)
        .open(device)?;
    // SAFETY: both are open descriptors; dup2 replaces stdin atomically.
    if unsafe { libc::dup2(tty.as_raw_fd(), libc::STDIN_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn main() -> Result<()> {
    // Errors go to stderr before the terminal is touched, so the program
    // that started zencode as $EDITOR can show them.
    let command = match cli::parse(std::env::args().skip(1), |p| p.exists()) {
        Ok(command) => command,
        Err(e) => {
            eprintln!("zencode: {e}\nRun `zencode --help` for usage.");
            std::process::exit(1);
        }
    };
    let (settings, tab_size, files) = match command {
        Command::Help => {
            println!("{}", usage());
            return Ok(());
        }
        Command::Version => {
            println!("{} {}", brand::full_name(), brand::VERSION);
            return Ok(());
        }
        Command::Edit {
            settings,
            tab_size,
            files,
        } => (settings, tab_size, files),
    };
    // zencode never invents a file name: no file means "just tell me
    // what you are".
    if files.is_empty() {
        println!("{} {}", brand::full_name(), brand::VERSION);
        return Ok(());
    }
    if let Some(n) = tab_size {
        text::set_tab_width(n);
    }

    // With `-`, read all of stdin first; keyboard input then comes from
    // the terminal. (On Windows crossterm reads CONIN$ anyway.)
    let mut stdin_text = String::new();
    if files.iter().any(|f| f.source == Source::Stdin) {
        io::stdin()
            .read_to_string(&mut stdin_text)
            .context("reading standard input")?;
        #[cfg(unix)]
        reattach_stdin().context("opening the terminal for keyboard input")?;
    }

    // Open everything up front, so a bad file is reported before the
    // screen changes.
    let count = files.len();
    let mut editors = Vec::with_capacity(count);
    for (i, file) in files.into_iter().enumerate() {
        let mut editor = Editor::new(file.source, &stdin_text, settings.clone())?;
        if count > 1 {
            editor.set_file_number(i + 1, count);
        }
        if let Some(goto) = &file.goto {
            editor.goto(goto);
        }
        editors.push(editor);
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    let _guard = TerminalGuard::enter(!settings.no_mouse)?;
    let mut out = BufWriter::with_capacity(1 << 16, io::stdout().lock());
    for mut editor in editors {
        editor.run(&mut out)?;
    }
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_alternate_screen_state() {
        assert!(alternate_screen_reported(b"\x1b[?1049;1$y\x1b[?62;22c"));
        assert!(alternate_screen_reported(b"\x1b[?1049;3$y"));
        assert!(!alternate_screen_reported(b"\x1b[?1049;2$y\x1b[?62;22c"));
        assert!(!alternate_screen_reported(b"\x1b[?1049;0$y"));
        // A terminal without DECRQM only answers the DA1 request.
        assert!(!alternate_screen_reported(b"\x1b[?62;22c"));
    }

    #[test]
    fn waits_for_device_attributes() {
        assert!(da1_complete(b"\x1b[?1049;1$y\x1b[?62;22c"));
        assert!(da1_complete(b"\x1b[?6c"));
        assert!(!da1_complete(b"\x1b[?1049;1$y"));
        assert!(!da1_complete(b"\x1b[?62;2"));
    }
}
