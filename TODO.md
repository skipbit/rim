# TODO List for rim

## Done

- **Search:** `/` for searching, `n` / `N` for next/previous match. *(`?` reverse search still TODO.)*
- **Text Objects:** `iw`, `aw`, `i"/a"`, `i(/a(` (`[ { b B`), `ip`, `ap` — with counts.
- **Motions:** `w`, `b`, `e`, `ge`, `W`, `B`, `E`, `gE`, `0`, `^`, `$`, `f`, `t`, `F`, `T`, `;`, `,`, `%`, `gg`, `G`, `<count>G`.
- **Operators:** `d`, `c`, `y` combined with motions and text objects, plus `dd/cc/yy` and `D/C/Y`; counts (`2d3w`).
- **Registers:** unnamed register with charwise/linewise `p`/`P`. *(Named registers still TODO.)*
- **Unicode:** rope buffer, grapheme-aware cursor, wide-character display columns.

## Vim/NeoVim Feature Differences (Editing)

- **Visual Mode:** Selection of text for copy, cut, paste operations.
- **Dot Repeat:** `.` repeating a full operator+motion / text-object command (currently only simple edits repeat).
- **Reverse Search:** `?`.
- **Replace:** `:%s/old/new/g` command.
- **Line Numbers:** Displaying absolute and relative line numbers.
- **Jumping:** `Ctrl-o`, `Ctrl-i` for navigation. *(`gg`, `G` done.)*
- **Change Case:** `~`, `gU`, `gu`.
- **Join Lines:** `J`.
- **Replace char:** `r`.
- **Sticky column:** keep the desired column across `j`/`k`.
- **Window Management:** Splitting windows, navigating between them.
- **Macros:** Recording and replaying key sequences.
- **Plugins:** Support for extending functionality via plugins.
- **Registers:** Storing text in named registers.
- **Marks:** Setting and jumping to specific positions.
- **Folding:** Collapsing and expanding code blocks.
- **Auto-indentation/Smart-indentation:** Automatic indentation based on file type.
- **Syntax Highlighting:** Coloring code based on syntax.
- **Completion:** Autocompletion for words, paths, etc.
- **File Explorer:** Built-in file browsing (e.g., Netrw).
- **Command-line History:** Recalling previous commands.
- **Jump List:** Navigating through recent cursor positions.
- **Change List:** Navigating through recent changes.
- **Insert Mode Completion:** `Ctrl-n`, `Ctrl-p`.
- **Abbreviation/Mapping:** Custom keybindings and text expansions.
- **Diff Mode:** Comparing files side-by-side.
- **Spell Check:** Built-in spell checking.
- **External Commands:** Running shell commands from within Vim.
- **Background Jobs:** Running tasks asynchronously.
- **Terminal Emulator:** Built-in terminal.
