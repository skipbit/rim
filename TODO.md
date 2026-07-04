# TODO List for rim

## Done

- **Search:** `/` for searching, `n` / `N` for next/previous match. *(`?` reverse search still TODO.)*
- **Text Objects:** `iw`, `aw`, `i"/a"`, `i(/a(` (`[ { b B`), `ip`, `ap` — with counts.
- **Motions:** `w`, `b`, `e`, `ge`, `W`, `B`, `E`, `gE`, `0`, `^`, `$`, `f`, `t`, `F`, `T`, `;`, `,`, `%`, `gg`, `G`, `<count>G`.
- **Operators:** `d`, `c`, `y` combined with motions and text objects, plus `dd/cc/yy` and `D/C/Y`; counts (`2d3w`).
- **Registers:** unnamed register with charwise/linewise `p`/`P`. *(Named registers still TODO.)*
- **Unicode:** rope buffer, grapheme-aware cursor, wide-character display columns.
- **Syntax Highlighting:** background tree-sitter highlighting of the visible window.
- **Line Numbers:** absolute line-number gutter (with an LSP diagnostic sign column). *(Relative numbers still TODO.)*
- **LSP (rust-analyzer):** diagnostics, hover (`K`), go-to-definition (`gd`), format (`:format`), rename (`:rename`), completion (`Ctrl-n`). *(Single buffer, Rust only; incremental sync + multi-language + multi-file rename still TODO.)*
- **Jumping:** `Ctrl-o` / `Ctrl-i` jump list (populated by `gd`). *(Full jump history across all motions still TODO.)*

## Vim/NeoVim Feature Differences (Editing)

- **Visual Mode:** Selection of text for copy, cut, paste operations.
- **Dot Repeat:** `.` repeating a full operator+motion / text-object command (currently only simple edits repeat).
- **Reverse Search:** `?`.
- **Replace:** `:%s/old/new/g` command.
- **Line Numbers:** Relative line numbers. *(Absolute line-number gutter done.)*
- **Jumping:** *(`Ctrl-o`, `Ctrl-i`, `gg`, `G` done; full jump history still TODO.)*
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
- **Syntax Highlighting:** *(tree-sitter highlighting done.)*
- **Completion:** *(LSP completion done; word/path completion still TODO.)*
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
