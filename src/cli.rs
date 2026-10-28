//! Command line parsing. zencode is often started by other programs as
//! `$EDITOR`, so it accepts nano's syntax (`+LINE,COL`, `+/STRING`,
//! bundled short options, `--`, `-` for stdin), VS Code's `-g FILE:LINE`,
//! and quietly ignores nano options it has no use for.

use std::path::{Path, PathBuf};

/// Behaviour switches shared by every file opened in one run.
#[derive(Clone, Default)]
pub struct Settings {
    pub sandboxed: bool,
    pub view: bool,
    pub save_on_exit: bool,
    pub tabs_to_spaces: bool,
    pub autoindent: bool,
    pub no_newlines: bool,
    pub unix: bool,
    pub no_help: bool,
    pub no_mouse: bool,
    pub no_read: bool,
    pub backup: bool,
    pub backup_dir: Option<PathBuf>,
    pub restricted: bool,
    pub cut_from_cursor: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    Path(PathBuf),
    Stdin,
}

/// Where the cursor starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Goto {
    /// 1-based; negative numbers count back from the end of the file or line.
    Line { line: i64, col: Option<i64> },
    Search {
        query: String,
        backward: bool,
        exact: bool,
        regex: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileArg {
    pub source: Source,
    pub goto: Option<Goto>,
}

pub enum Command {
    Help,
    Version,
    Edit {
        settings: Settings,
        tab_size: Option<usize>,
        files: Vec<FileArg>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Opt {
    Help,
    Version,
    Sandboxed,
    ColonParsing,
    View,
    SaveOnExit,
    TabSize,
    TabsToSpaces,
    AutoIndent,
    NoNewlines,
    Unix,
    NoHelp,
    NoMouse,
    NoRead,
    Backup,
    BackupDir,
    Restricted,
    CutFromCursor,
    /// Accepted and ignored.
    Ignore,
    /// Accepted and ignored, along with its argument.
    IgnoreArg,
}

impl Opt {
    fn takes_arg(self) -> bool {
        matches!(self, Opt::TabSize | Opt::BackupDir | Opt::IgnoreArg)
    }
}

const OPTIONS: &[(Option<char>, &str, Opt)] = &[
    (Some('h'), "help", Opt::Help),
    (Some('V'), "version", Opt::Version),
    // nano's -s is --speller; zencode keeps it for the sandbox.
    (Some('s'), "sandboxed", Opt::Sandboxed),
    (Some('@'), "colonparsing", Opt::ColonParsing),
    // VS Code's -g / --goto, sent by tools that take zencode for VS Code.
    // (nano's -g is --showcursor.)
    (Some('g'), "goto", Opt::ColonParsing),
    (Some('v'), "view", Opt::View),
    (Some('t'), "saveonexit", Opt::SaveOnExit),
    (Some('T'), "tabsize", Opt::TabSize),
    (Some('E'), "tabstospaces", Opt::TabsToSpaces),
    (Some('i'), "autoindent", Opt::AutoIndent),
    (Some('L'), "nonewlines", Opt::NoNewlines),
    (Some('u'), "unix", Opt::Unix),
    (Some('x'), "nohelp", Opt::NoHelp),
    (None, "nomouse", Opt::NoMouse),
    (Some('n'), "noread", Opt::NoRead),
    (Some('B'), "backup", Opt::Backup),
    (Some('C'), "backupdir", Opt::BackupDir),
    (Some('R'), "restricted", Opt::Restricted),
    (Some('k'), "cutfromcursor", Opt::CutFromCursor),
    // What zencode already does anyway.
    (Some('l'), "linenumbers", Opt::Ignore),
    (Some('N'), "noconvert", Opt::Ignore),
    (Some('c'), "constantshow", Opt::Ignore),
    (Some('m'), "mouse", Opt::Ignore),
    (Some('Z'), "zap", Opt::Ignore),
    // nano features zencode doesn't have.
    (Some('S'), "softwrap", Opt::Ignore),
    (Some('0'), "zero", Opt::Ignore),
    (Some('G'), "locking", Opt::Ignore),
    (Some('P'), "positionlog", Opt::Ignore),
    (Some('A'), "smarthome", Opt::Ignore),
    (Some('D'), "boldtext", Opt::Ignore),
    (Some('F'), "multibuffer", Opt::Ignore),
    (Some('H'), "historylog", Opt::Ignore),
    (Some('I'), "ignorercfiles", Opt::Ignore),
    (Some('K'), "rawsequences", Opt::Ignore),
    (Some('M'), "trimblanks", Opt::Ignore),
    (Some('O'), "bookstyle", Opt::Ignore),
    (Some('U'), "quickblank", Opt::Ignore),
    (Some('W'), "wordbounds", Opt::Ignore),
    (Some('a'), "atblanks", Opt::Ignore),
    (Some('b'), "breaklonglines", Opt::Ignore),
    (Some('d'), "rebinddelete", Opt::Ignore),
    (Some('e'), "emptyline", Opt::Ignore),
    (Some('j'), "jumpyscrolling", Opt::Ignore),
    (Some('p'), "preserve", Opt::Ignore),
    (Some('q'), "indicator", Opt::Ignore),
    (Some('w'), "nowrap", Opt::Ignore),
    (Some('y'), "afterends", Opt::Ignore),
    (Some('z'), "listsyntaxes", Opt::Ignore),
    (Some('%'), "stateflags", Opt::Ignore),
    (Some('_'), "minibar", Opt::Ignore),
    (Some('1'), "solosidescroll", Opt::Ignore),
    (Some('/'), "modernbindings", Opt::Ignore),
    (Some('!'), "magic", Opt::Ignore),
    (None, "showcursor", Opt::Ignore),
    (Some('f'), "rcfile", Opt::IgnoreArg),
    (Some('J'), "guidestripe", Opt::IgnoreArg),
    (Some('Q'), "quotestr", Opt::IgnoreArg),
    (Some('X'), "wordchars", Opt::IgnoreArg),
    (Some('Y'), "syntax", Opt::IgnoreArg),
    (Some('r'), "fill", Opt::IgnoreArg),
    (Some('o'), "operatingdir", Opt::IgnoreArg),
    (None, "speller", Opt::IgnoreArg),
    // Added by tools that expect a GUI editor.
    (None, "wait", Opt::Ignore),
    (None, "new-window", Opt::Ignore),
    (None, "reuse-window", Opt::Ignore),
];

/// Parse the arguments after the program name. `exists` tells whether a
/// file is there, for `FILE:LINE` parsing.
pub fn parse(
    args: impl IntoIterator<Item = String>,
    exists: impl Fn(&Path) -> bool,
) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut settings = Settings::default();
    let mut tab_size = None;
    let mut colon_parsing = false;
    let mut names = Vec::new();
    let mut goto = None;
    let mut options_done = false;

    while let Some(arg) = args.next() {
        if options_done || arg == "-" || !arg.starts_with(['-', '+']) {
            names.push((arg, goto.take()));
            continue;
        }
        if arg == "--" {
            options_done = true;
            continue;
        }
        if let Some(spec) = arg.strip_prefix('+') {
            goto = Some(parse_goto(spec).ok_or_else(|| format!("invalid position '{arg}'"))?);
            continue;
        }

        // Each entry: the option and its argument, if it takes one.
        let mut found: Vec<(Opt, Option<String>)> = Vec::new();
        if let Some(long) = arg.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let opt = OPTIONS
                .iter()
                .find(|o| o.1 == name)
                .map(|o| o.2)
                .ok_or_else(|| format!("unknown option '--{name}'"))?;
            let value = match (opt.takes_arg(), value) {
                (true, Some(v)) => Some(v),
                (true, None) => Some(
                    args.next()
                        .ok_or_else(|| format!("option '--{name}' needs an argument"))?,
                ),
                (false, Some(_)) => return Err(format!("option '--{name}' takes no argument")),
                (false, None) => None,
            };
            found.push((opt, value));
        } else {
            // Bundled short options, like -liT4 or -iT 4.
            let shorts = &arg[1..];
            for (i, c) in shorts.char_indices() {
                let opt = OPTIONS
                    .iter()
                    .find(|o| o.0 == Some(c))
                    .map(|o| o.2)
                    .ok_or_else(|| format!("unknown option '-{c}'"))?;
                if opt.takes_arg() {
                    let rest = &shorts[i + c.len_utf8()..];
                    let value = if rest.is_empty() {
                        args.next()
                            .ok_or_else(|| format!("option '-{c}' needs an argument"))?
                    } else {
                        rest.to_string()
                    };
                    found.push((opt, Some(value)));
                    break;
                }
                found.push((opt, None));
            }
        }

        for (opt, value) in found {
            match opt {
                Opt::Help => return Ok(Command::Help),
                Opt::Version => return Ok(Command::Version),
                Opt::Sandboxed => settings.sandboxed = true,
                Opt::ColonParsing => colon_parsing = true,
                Opt::View => settings.view = true,
                Opt::SaveOnExit => settings.save_on_exit = true,
                Opt::TabSize => {
                    let v = value.unwrap_or_default();
                    match v.parse::<usize>() {
                        Ok(n) if n > 0 => tab_size = Some(n),
                        _ => return Err(format!("invalid tab size '{v}'")),
                    }
                }
                Opt::TabsToSpaces => settings.tabs_to_spaces = true,
                Opt::AutoIndent => settings.autoindent = true,
                Opt::NoNewlines => settings.no_newlines = true,
                Opt::Unix => settings.unix = true,
                Opt::NoHelp => settings.no_help = true,
                Opt::NoMouse => settings.no_mouse = true,
                Opt::NoRead => settings.no_read = true,
                Opt::Backup => settings.backup = true,
                Opt::BackupDir => {
                    settings.backup = true;
                    settings.backup_dir = value.map(PathBuf::from);
                }
                Opt::Restricted => settings.restricted = true,
                Opt::CutFromCursor => settings.cut_from_cursor = true,
                Opt::Ignore | Opt::IgnoreArg => {}
            }
        }
    }

    let mut files = Vec::new();
    for (name, goto) in names {
        if name == "-" && !options_done {
            if files.iter().any(|f: &FileArg| f.source == Source::Stdin) {
                return Err("'-' (standard input) can only be given once".into());
            }
            files.push(FileArg {
                source: Source::Stdin,
                goto,
            });
            continue;
        }
        let (path, position) = split_position(&name, colon_parsing, &exists);
        files.push(FileArg {
            source: Source::Path(PathBuf::from(path)),
            goto: position.or(goto),
        });
    }
    Ok(Command::Edit {
        settings,
        tab_size,
        files,
    })
}

/// The part after `+`: `LINE[,COLUMN]`, or `[crCR]/STRING` / `[crCR]?STRING`.
fn parse_goto(spec: &str) -> Option<Goto> {
    if let Some(i) = spec.find(['/', '?']) {
        let (flags, rest) = spec.split_at(i);
        if flags.chars().all(|c| "crCR".contains(c)) {
            let (mut exact, mut regex) = (false, false);
            for c in flags.chars() {
                match c {
                    'c' => exact = true,
                    'C' => exact = false,
                    'r' => regex = true,
                    _ => regex = false,
                }
            }
            return Some(Goto::Search {
                query: rest[1..].to_string(),
                backward: rest.starts_with('?'),
                exact,
                regex,
            });
        }
    }
    let (line, col) = match spec.split_once(',') {
        Some((l, c)) => (l, Some(c)),
        None => (spec, None),
    };
    let number = |s: &str| {
        let digits = s.strip_prefix('-').unwrap_or(s);
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<i64>().ok())
            .flatten()
    };
    Some(Goto::Line {
        line: number(line)?,
        col: match col {
            Some(c) => Some(number(c)?),
            None => None,
        },
    })
}

/// `NAME:LINE[:COL]` handling, following nano's `--colonparsing`: a
/// trailing `:digits` is only taken as a position when no file with the
/// literal name exists. With `force` (`-@`, `-g`) it always is then;
/// without, only when that leaves the name of a file that does exist, so
/// a new file named `notes:1` can still be created.
fn split_position(
    name: &str,
    force: bool,
    exists: &impl Fn(&Path) -> bool,
) -> (String, Option<Goto>) {
    let snip = |s: &str| -> Option<(String, i64)> {
        let (head, tail) = s.rsplit_once(':')?;
        if head.is_empty() || tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some((head.to_string(), tail.parse().ok()?))
    };
    let line = |line, col| Some(Goto::Line { line, col });

    if exists(Path::new(name)) {
        return (name.to_string(), None);
    }
    let Some((head, a)) = snip(name) else {
        return (name.to_string(), None);
    };
    if exists(Path::new(&head)) {
        return (head, line(a, None));
    }
    if let Some((base, b)) = snip(&head)
        && (force || exists(Path::new(&base)))
    {
        return (base, line(b, Some(a)));
    }
    if force {
        return (head, line(a, None));
    }
    (name.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(args: &[&str], existing: &[&str]) -> Vec<FileArg> {
        let args = args.iter().map(|s| s.to_string());
        match parse(args, |p| existing.iter().any(|e| Path::new(e) == p)) {
            Ok(Command::Edit { files, .. }) => files,
            Ok(_) => panic!("not an edit command"),
            Err(e) => panic!("{e}"),
        }
    }

    fn settings(args: &[&str]) -> (Settings, Option<usize>) {
        match parse(args.iter().map(|s| s.to_string()), |_| false) {
            Ok(Command::Edit {
                settings, tab_size, ..
            }) => (settings, tab_size),
            _ => panic!("not an edit command"),
        }
    }

    fn path(p: &str, goto: Option<Goto>) -> FileArg {
        FileArg {
            source: Source::Path(PathBuf::from(p)),
            goto,
        }
    }

    fn line(line: i64, col: Option<i64>) -> Option<Goto> {
        Some(Goto::Line { line, col })
    }

    #[test]
    fn plus_line_and_column() {
        assert_eq!(files(&["+5", "a"], &[]), [path("a", line(5, None))]);
        assert_eq!(files(&["+5,3", "a"], &[]), [path("a", line(5, Some(3)))]);
        assert_eq!(files(&["+-1", "a"], &[]), [path("a", line(-1, None))]);
        assert_eq!(files(&["+2,-1", "a"], &[]), [path("a", line(2, Some(-1)))]);
        // Each +LINE belongs to the file after it.
        assert_eq!(
            files(&["+3", "a", "b", "+4", "c"], &[]),
            [
                path("a", line(3, None)),
                path("b", None),
                path("c", line(4, None))
            ]
        );
        let args = ["+x", "a"].map(String::from);
        assert!(parse(args, |_| false).is_err());
    }

    #[test]
    fn plus_search() {
        let search = |q: &str, backward, exact, regex| {
            Some(Goto::Search {
                query: q.into(),
                backward,
                exact,
                regex,
            })
        };
        assert_eq!(
            files(&["+/foo", "a"], &[]),
            [path("a", search("foo", false, false, false))]
        );
        assert_eq!(
            files(&["+c/Foo", "a"], &[]),
            [path("a", search("Foo", false, true, false))]
        );
        assert_eq!(
            files(&["+cC?a,b", "a"], &[]),
            [path("a", search("a,b", true, false, false))]
        );
        assert_eq!(
            files(&["+r/x/y", "a"], &[]),
            [path("a", search("x/y", false, false, true))]
        );
    }

    #[test]
    fn colon_positions() {
        // -g as sent by tools that take zencode for VS Code.
        assert_eq!(
            files(&["-g", "/tmp/x.md:1"], &["/tmp/x.md"]),
            [path("/tmp/x.md", line(1, None))]
        );
        assert_eq!(
            files(&["--goto", "x.md:7:3"], &[]),
            [path("x.md", line(7, Some(3)))]
        );
        assert_eq!(files(&["-@", "new:2"], &[]), [path("new", line(2, None))]);
        // Without -@ only when the rest names an existing file.
        assert_eq!(files(&["x.md:5"], &["x.md"]), [path("x.md", line(5, None))]);
        assert_eq!(
            files(&["x.md:5:2"], &["x.md"]),
            [path("x.md", line(5, Some(2)))]
        );
        assert_eq!(files(&["new:1"], &[]), [path("new:1", None)]);
        // A file with the literal name wins.
        assert_eq!(files(&["-@", "a:1"], &["a:1", "a"]), [path("a:1", None)]);
    }

    #[test]
    fn end_of_options_and_stdin() {
        assert_eq!(files(&["--", "-file"], &[]), [path("-file", None)]);
        assert_eq!(
            files(&["--", "+5", "-"], &[]),
            [path("+5", None), path("-", None)]
        );
        assert_eq!(
            files(&["+2", "-"], &[]),
            [FileArg {
                source: Source::Stdin,
                goto: line(2, None)
            }]
        );
        assert!(parse(["-", "-"].map(String::from), |_| false).is_err());
    }

    #[test]
    fn bundled_and_long_options() {
        let (s, tab) = settings(&["-lviT8", "f"]);
        assert!(s.view && s.autoindent && !s.sandboxed);
        assert_eq!(tab, Some(8));
        let (_, tab) = settings(&["-T", "2", "f"]);
        assert_eq!(tab, Some(2));
        let (s, tab) = settings(&["--tabsize=3", "--saveonexit", "--sandboxed", "f"]);
        assert!(s.save_on_exit && s.sandboxed);
        assert_eq!(tab, Some(3));
        // Ignored options still consume their argument.
        assert_eq!(
            files(&["-Y", "rust", "--speller", "aspell", "f"], &[]),
            [path("f", None)]
        );
        let (s, _) = settings(&["-C", "/tmp/bk", "f"]);
        assert!(s.backup && s.backup_dir == Some(PathBuf::from("/tmp/bk")));
    }

    #[test]
    fn unknown_options_are_errors() {
        for bad in ["-q9", "--bogus", "-T0", "--view=1"] {
            let args = [bad, "f"].map(String::from);
            assert!(parse(args, |_| false).is_err(), "{bad}");
        }
        // -w and --wait come from tools expecting a GUI editor.
        assert_eq!(files(&["-w", "--wait", "f"], &[]), [path("f", None)]);
    }
}
