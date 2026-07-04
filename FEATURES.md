# rim Editor Features

This document outlines the current features and keybindings of the `rim` terminal editor.

`rim` uses a rope-backed text buffer and a Unicode-correct cursor (char/grapheme
coordinates, wide-character-aware display columns), with invertible-transaction
undo and viewport scrolling. It runs on an asynchronous (tokio) event loop, so
background work never blocks input, and it renders **tree-sitter** syntax
highlighting for the visible window. A line-number gutter runs down the left
edge, and an embedded **LSP client** (rust-analyzer) provides diagnostics,
hover, go-to-definition, formatting, rename, and completion.

## Syntax Highlighting

`rim` highlights source code using [tree-sitter](https://tree-sitter.github.io/).

-   Rust is highlighted out of the box (keywords, functions, types, strings,
    comments, constants, punctuation, …).
-   Parsing runs on a **background thread**: edits are debounced (~30 ms) and
    re-highlighted off the main loop, so typing never blocks on the parser.
    Results are revision-tracked — stale results are dropped, and the previous
    colours stay on screen until fresh ones arrive (no flash of unstyled text).
-   Only the visible window is coloured; the previous plain rendering is used as
    a fast path until the first highlights arrive (or if the grammar fails to
    load, in which case the editor keeps working without colour).

## Language Intelligence (LSP)

`rim` embeds a Language Server Protocol client. For Rust files it launches
[`rust-analyzer`](https://rust-analyzer.github.io/) (found on `PATH`) the first
time a `.rs` file is opened; if the server is not installed the editor keeps
working without language features.

-   **Diagnostics**: errors and warnings are shown as coloured underlines, with
    a severity sign (`E`/`W`) in the gutter and the message for the diagnostic
    under the cursor on the status line. They update as you type (debounced) and
    clear when fixed.
-   **Hover** (`K`): show the type / documentation for the symbol under the
    cursor in a popup. Any key dismisses it.
-   **Go-to-definition** (`gd`): jump to a symbol's definition, in the same file
    or another (which is opened automatically). Use `Ctrl-o` / `Ctrl-i` to jump
    back / forward through the jump list.
-   **Format** (`:format` / `:fmt`): reformat the whole document; a single `u`
    reverts it.
-   **Rename** (`:rename <new>`): rename the symbol under the cursor. Edits in
    the current file are applied as one undo step; edits in other files are
    reported but not applied (single-buffer editor).
-   **Completion** (Insert mode, `Ctrl-n` or `Ctrl-Space`): open a completion
    menu that filters as you type. `Ctrl-n` / `Ctrl-p` (or `↓` / `↑`) move the
    selection, `Enter` / `Tab` accept, `Esc` dismisses.

Requests run on background tasks, so the editor never blocks on the server. The
whole document is synced on each debounced edit; the line/column mapping honours
the position encoding negotiated with the server (UTF-8 preferred, UTF-16
fallback), so multibyte text stays aligned.

## Modes

`rim` operates in different modes, similar to Vim/NeoVim, to provide a powerful and efficient editing experience.

### 1. Normal Mode

The default mode. Navigate the file and issue commands. Most commands accept a
leading **count** (e.g. `3w`, `2dd`, `d2w`).

**Motions** (move the cursor; also usable as the range for an operator):

-   `h` `j` `k` `l` / Arrow Keys: Move left / down / up / right (`h`/`l` move by grapheme cluster)
-   `w` / `W`: Next word / WORD start
-   `b` / `B`: Previous word / WORD start
-   `e` / `E`: Next word / WORD end
-   `ge` / `gE`: Previous word / WORD end
-   `0`: First column
-   `^`: First non-blank character
-   `$`: End of line
-   `gg`: First line (`<count>gg` → line _count_)
-   `G`: Last line (`<count>G` → line _count_)
-   `f{char}` / `F{char}`: To next / previous occurrence of `{char}` on the line
-   `t{char}` / `T{char}`: Till before next / after previous `{char}`
-   `;` / `,`: Repeat the last `f`/`t`/`F`/`T` in the same / opposite direction
-   `%`: Jump to the matching bracket

**Operators** (apply to a motion or text object; e.g. `dw`, `c$`, `y%`, `d2w`):

-   `d`: Delete
-   `c`: Change (delete, then enter Insert Mode; `cw` acts like `ce`)
-   `y`: Yank (copy)
-   Doubled — `dd` / `cc` / `yy`: Operate on whole line(s) (linewise)
-   `D` / `C` / `Y`: Delete / change to end of line, yank line

**Text objects** (after an operator, with `i` = inner or `a` = around; e.g. `diw`, `ci"`, `da(`, `dip`):

-   `iw` / `aw`: Inner / a word (`W` for WORD)
-   `i"` `i'` `` i` ``: Inside quotes (and `a"` etc. to include them)
-   `i(` `i[` `i{` (also `ib` / `iB`): Inside brackets (and `a(` etc. to include them)
-   `ip` / `ap`: Inner / a paragraph (linewise)
-   Counts extend the object (e.g. `2iw`, `2i(` selects the next enclosing pair)

**Editing & other:**

-   `i` / `a`: Enter Insert Mode at / after the cursor
-   `A` / `I`: Insert at end of line / at first non-blank
-   `o` / `O`: Open a new line below / above and enter Insert Mode
-   `x`: Delete character(s) under the cursor (fills the register; count-aware)
-   `p` / `P`: Paste the register after / before the cursor (charwise or linewise; count-aware)
-   `u`: Undo last change
-   `Ctrl-r`: Redo last undone change
-   `.`: Repeat the last change *(currently repeats simple single-key edits; repeating a full operator+motion command is planned)*
-   `/`: Enter Search Mode; `n` / `N`: next / previous match
-   `gd`: Go to definition (LSP); `K`: Hover (LSP)
-   `Ctrl-o` / `Ctrl-i`: Jump back / forward through the jump list
-   `:`: Enter Command Mode
-   `q`: Quit the editor

### 2. Insert Mode

In Insert Mode, you can type and modify the content of the file.

**Keybindings:**

-   `Esc`: Exit Insert Mode and return to **Normal Mode**
-   Typing characters: Inserts characters at the cursor position
-   `Enter`: Inserts a new line
-   `Backspace`: Deletes the character before the cursor
-   Arrow Keys: Move cursor (Left, Down, Up, Right)
-   `Ctrl-n` / `Ctrl-Space`: Open LSP completion (then `Ctrl-n` / `Ctrl-p` to
    select, `Enter` / `Tab` to accept, `Esc` to dismiss)

### 3. Command Mode

In Command Mode, you can execute various editor commands by typing them at the prompt (indicated by a `:` at the bottom of the screen).

**Keybindings:**

-   `Esc`: Exit Command Mode and return to **Normal Mode**
-   Typing characters: Builds the command string
-   `Backspace`: Deletes the last character in the command string
-   `Enter`: Executes the typed command

**Available Commands:**

-   `:w` or `:write [filename]`
    -   Saves the current file.
    -   If `[filename]` is provided, saves the file to the specified path. This is used for saving new files or saving an existing file to a new location.
-   `:q` or `:quit`
    -   Exits the editor.
    -   If there are unsaved changes, the editor will prevent quitting.
-   `:e` or `:edit <filename>`
    -   Opens the specified file in the editor.
    -   Replaces the current buffer with the content of the new file.
-   `:format` or `:fmt`
    -   Reformats the whole document via the language server (one undo step).
-   `:rename <new>`
    -   Renames the symbol under the cursor via the language server.

### 4. Search Mode

Entered with `/` from Normal Mode. Type a query and press `Enter` to jump to the
first match; `Esc` cancels. Use `n` / `N` in Normal Mode to cycle matches.

## New File Creation

You can start `rim` without any arguments (e.g., just `cargo run`). This will open an empty buffer. You can then type your content and save it as a new file using the `:w <filename>` command.
