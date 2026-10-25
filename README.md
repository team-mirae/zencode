# zencode · 젠코드 · ゼンコード

A nano-style terminal text editor that handles 全角 (zenkaku / full-width)
text properly: 한글, 日本語 and other wide characters take up two columns,
and the cursor, scrolling and line clipping all account for that.

Runs on Linux, macOS and Windows. Rendering goes through
[crossterm](https://github.com/crossterm-rs/crossterm).

## Install

On Linux or macOS, including over ssh:

```sh
curl -fsSL https://raw.githubusercontent.com/team-mirae/zencode/main/install.sh | sh
```

This installs the latest release into `~/.local/bin`, with no sudo needed,
and verifies its SHA-256 checksum. To pin a release or pick the directory,
set the variables on the `sh` side of the pipe:

```sh
curl -fsSL https://raw.githubusercontent.com/team-mirae/zencode/main/install.sh | ZENCODE_VERSION=v0.1.0 ZENCODE_INSTALL_DIR=/usr/local/bin sh
```
 Windows builds are on the
[releases page](https://github.com/team-mirae/zencode/releases).

From source:

```sh
cargo install --git https://github.com/team-mirae/zencode
```

## Usage

```sh
zencode path/to/file.txt      # created if it doesn't exist
zencode -s secrets.env        # sandboxed clipboard
```

For development:

```sh
cargo run --release -- path/to/file.txt      # created if it doesn't exist
cargo run --release -- -s secrets.env        # sandboxed clipboard
```

Running `zencode` with no file prints the version, since zencode never
picks a file name for you.

## Keys

The full spec is in [KEYBINDS.md](KEYBINDS.md). Every major key is also
shown in the shortcut bar at the bottom of the screen.

| Key               | Action                       |
|-------------------|------------------------------|
| ⌃X / ⌃⇧X          | Exit / exit without warning  |
| ⌃S / ⌃O           | Save / export to a new file  |
| ⌃C / ⌃B / ⌃V      | Copy / extract (cut) / paste |
| ⌃A                | Select all                   |
| ⌃E or ⌥E          | Undo                         |
| ⌃R or ⌥R          | Redo                         |
| ⇧ + movement      | Extend the selection         |
| ⌃←→ or ⌥←→        | Move by letterblock          |
| ⌃⌫ or ⌥⌫          | Delete letterblock left      |
| ⌃⌦ or ⌥⌦          | Delete letterblock right     |
| ⌃↑↓ or ⌥↑↓        | Move line up / down          |
| ⌃F / ⌃G           | Find / find exactly          |
| Tab / ⇧Tab        | Next / previous match        |
| ⌃/ or F1          | Help                         |

With nothing selected, ⌃C and ⌃B act on the whole line.

Undo works in steps. A run of typing, backspacing or line moves at one spot
is one step, like an insert-mode run in vim. Enter, paste and extract are
steps of their own, and moving the cursor starts a new one.

### Letterblocks

A letterblock is a block of text with its own meaning: a word in English,
a run of one script in Japanese (あなたの|ギター|になりたい|！), and in
Korean a stem with its particle or ending split off (학교|에서는,
먹|었습니다). ⌃/⌥ + ←→ moves by letterblock (add ⇧ to select), and
⌃/⌥ + ⌫⌦ deletes one.

The Korean split uses a short list of common particles and endings,
checked against the syllable before them (가을 and 사과 stay whole). It
has no dictionary, so it still gets some nouns wrong (고양이 → 고양|이).

Terminals often send ⌃⌫ as ⌃H, and iTerm2's "natural text editing"
preset sends ⌥⌫ as ⌃W and ⌥⌦ as ⌥D; zencode treats all of these as
letterblock deletes.

### Mouse

Click to place the cursor, ⇧+click or drag to select, double-click to
select a letterblock (the same rules as ⌃/⌥ + ←→), and triple-click to
select the line. The wheel scrolls the view without moving the cursor.

Shortcuts in the bar can be clicked too, and highlight when the pointer is
over them: Exit, Save, Export, Find, Find Exact, Help, Copy, Extract,
Paste, Select All, Undo and Redo. Force Exit is deliberately not
clickable, and movement keys like Letterblock and Move Line are only hints.

zencode captures the mouse, so for your terminal's own text selection hold
⌥ (iTerm2, Terminal.app) or ⇧ (most Linux terminals) while dragging.

### Search

⌃F searches permissively and ⌃G searches exactly. Matches highlight as
you type, Tab and ⇧Tab step through them, Enter keeps the current one
selected, and Esc goes back to where you were.

A permissive search:

- ignores case, and matches full-width ＡＢＣ to ABC
- matches 초성: ㅎㄱ finds 한국, and 한구 already finds 한국 while you type
- matches ひらがな and カタカナ to each other, and turns a kana query into
  kanji: しんじゅく finds 新宿

Kana-to-kanji uses the dictionary in
[inputx-nihongo](https://crates.io/crates/inputx-nihongo) (MIT OR
Apache-2.0, about 27,000 compounds). It looks up whole words only, not
verb endings. 초성 matching needs no dictionary.

### Clipboard

Copies go to the system clipboard through the terminal (OSC 52). This
works over `ssh` in terminals that support it, such as iTerm2, WezTerm,
kitty, Ghostty, foot and Windows Terminal. ⌃V pastes what you copied in
zencode; to paste from other apps, use your terminal's own paste (⌘V or
⌃⇧V).

`-s` / `--sandboxed` turns both directions off: nothing is sent to the
system clipboard, and pastes from the terminal are refused.

### ⌃⇧X

Most terminals send ⌃⇧X exactly like ⌃X. zencode can only tell them apart
in terminals that support the kitty keyboard protocol (kitty, WezTerm,
Ghostty, foot, Alacritty, recent iTerm2). Elsewhere ⌃⇧X behaves like ⌃X
and still asks before discarding changes.

## Colors

zencode picks the richest color mode the terminal supports: 24-bit
truecolor, then 256 colors, then 16, then none. Every color is defined once
in RGB and mapped down to the closest match for the mode in use.

| Variable                   | Effect                                                  |
|----------------------------|---------------------------------------------------------|
| `ZENCODE_COLOR`            | Force `truecolor`, `256`, `16` or `none`                |
| `ZENCODE_AMBIGUOUS_WIDE=1` | Treat East Asian *ambiguous-width* characters as 2 cols |
| `NO_COLOR`                 | Disable colors                                          |

## Fonts

A terminal program can't pick its own font. The terminal emulator draws
every glyph, so to see Korean and Japanese text you need to configure a font
that covers both scripts in the terminal itself. Good choices:

- **Sarasa Mono K / J**: monospaced, with CJK glyphs exactly two columns
  wide.
- **Pretendard JP**: covers Latin, Hangul and Japanese, but it isn't
  monospaced. Use it as a fallback font behind a monospaced Latin font.
