# Keybinds

All keybinds in zencode are optimized for the HHKB JIS layout, especially for users who type:

- **English** in **QWERTY** or *QWERTY-shortcut AKLs*
- **Korean** in [**JIS Korean**](https://github.com/team-mirae/jis-korean/) or *KTS (두벌식)*
- **Japanese** in **Kana (かな入力)**

Support for other non-latin languages are out of scope.

## Core workflow

The core principle behind zencode is **read, then write**. macOS UX design insists
on creating the file first, using an application, then saving to disk. zencode does the opposite.

A file is created before zencode is launched. If the filename provided as the
launch argument does not exist, zencode will create it for you.

If no filename is specified, zencode will not start. We will not invent arbitrary filenames
for you. Your files should be named by you, not a program's suggestions.

Just running `zencode` is treated equally as `zencode --version`.

## Core principle

We try to preserve as much as we can from `nano` - the program which inspired this
project in the first place. However, it should be both **macOS-friendly** and **linux-friendly**, with macOS shortcuts taking priority.

Thus, navigate word-by-word also takes option(`⌥`) as its modifier. Some terminals
may not support the passthrough of the option key. zencode does not work around this.
Instead, we provide multiple shortcuts for the same operation, to maintain native-like
user experiences for both maxOS and linux.

Windows-specific keybinds are out of scope, however they should mostly work with linux.

## Beautiful and easy to use

Using a text editor should be intuitive. We believe the best instruction manual is
an instruction manual always visible. All major keybinds are always visible, with their alternative keybinds as well.

## Multiple points of access

Every keybind which is not on the control(`⌃`) layer should have more than one
keybind, on different modifier layers. This is to ensure that every operation is
possible on every supported operating system.

## Exiting zencode

Exiting a text editor should be easy. It follows `nano` conventions.

- `⌃X` to **Exit** gracefully. Unsaved changes will prompt a warning.
- `⌃⇧X` to **Exit without warning**.

## Disk I/O

A file is read by launching the program via `zencode filename.extension`.
There is no read operation required by the user.

Saving follows `nano` conventions.

- `⌃O` to **Export** to new file.
- `⌃S` to **Save** to current file.

## Clipboard access

zencode tries to use your terminal's clipboard. If your terminal supports system clipboard
access, it will automatically transfer to the system clipboard. If you do not want
this for security reasons, zencode can be run sandboxed, so no content ever leaves
zencode's memory.

Run `zencode --sandboxed filename.extension` or `zencode -s filename.extension`.

This also prevents inbound, so even if your terminal is wired up properly, zencode
will refuse to read your system's clipboard. This is useful for using zencode in
remote machines via `ssh`, when editing credentials or secrets.

Clipboard access follows modern conventions.

- `⌃C` to **Copy** to clipboard.
- `⌃V` to **Paste** from clipboard.

In zencode, X is a dangerous keybind meaning exit, so we avoided allocating it to cut.
Instead, we allocated cut to B. It does not stand for anything.

- `⌃B` to **Extract** to clipboard and delete from zencode.

## Selection control

zencode follows modern text editor conventions for selection control.

- `⌃A` to **Select All** contents of the current file.
- `⇧←` to **Expand Selection Leftward**s by one character.
- `⇧→` to **Expand Selection Rightward**s by one character.
- `⇧↑` to **Expand Selection Upward**s by one line.
- `⇧↓` to **Expand Selection Downward**s by one line.

## Caret navigation

zencode follows linux's approach to navigating the caret word-by-word. While it
would be optimal to use macOS's option(`⌥`) as the primary modifier for this layer,
some terminals swallow it and do not send it to the program behind the CLI.

Thus, control(`⌃`) acts as the word-by-word caret navigation accessor.
Option(`⌥`) will still work, if the terminal passes it through.

- `⌃←` or `⌥←` to **Move Caret Leftward**s by one continuous block of text.
- `⌃→` or `⌥→` to **Move Caret Rightward**s by one continuous block of text.

## Swapping lines

zencode offers the ability to swap lines with its upper or lower neighbors.

- `⌃↑` or `⌥↑` to **Move Line Upward**s and move that line to where this is.
- `⌃↓` or `⌥↓` to **Move Line Downward**s and move that line to where this is.

## State reversal and restoration

State management is done via undo and redo keybinds. Redo was inspired from `vim`,
which I still can't exit without Claude.

- `⌃E` or `⌥E` to **Undo** the recent edit.
- `⌃R` or `⌥R` to **Redo** the recent edit.

Where your terminal swallows option(`⌥`), control(`⌃`) does the same.

## Letterblock deletion

We use the term `letterblock` to mean a continuous block of text. The term word
is misleading to use when deleting a letterblock, as word is not synonymous to
*a continuous sequence of text which is not delimited by blankspace*, the moment
you leave English.

For example, how many words does `아마존웹서비시즈유한책임회사` contain? The answer is
"it depends." It gets worse in Japanese. Try editing `あなたのギターになりたい！` using
conventional `⌥+delete` or `⌃+backspace` of English-centric editors. Good luck.

A letterblock means a block of text with distinct meaning. For English, this is
synonoymous with the definition of a word. For Korean, this means component(어간)
and particle(어미), as well as blankspace. For Japanese, this means script borders
and blankspace.

- `⌃+backspace` or `⌥+backspace` to **Delete Letterblock Left**wards, until the termination of the first letterblock encountered.
- `⌃+delete` or `⌥+delete` to **Delete Letterblock Right**wards, until the termination of the first letterblock.

## Search contents

zencode is a true multi-language text editor. For the three languages it supports,
it offers smart search which actually saves you time.

- `⌃F` to **Find** something via *permissive search* in the file.
- `⌃G` to **Find Exact**ly, something via *strict search*, in the file.

A **permissive search** allows:

- For English: case-insensitive matching
- For Korean: initial matching (초성검색)
- For Japanese: kana searching (かな検索)

Your hiragana and katakana will be converted live into kanji by the editor, then it
will search the file for all matching candidates. Romaji is not converted live;
only finished かな will find a kanji candidate. Press `F6` in your IME to convert
to hiragana.

A **strict search** does not allow permissive matching.
