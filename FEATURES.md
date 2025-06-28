# rim Editor Features

This document outlines the current features and keybindings of the `rim` terminal editor.

## Modes

`rim` operates in different modes, similar to Vim/NeoVim, to provide a powerful and efficient editing experience.

### 1. Normal Mode

This is the default mode when you open or switch back to the editor. In Normal Mode, you can navigate the file and issue commands.

**Keybindings:**

-   `h`: Move cursor left
-   `j`: Move cursor down
-   `k`: Move cursor up
-   `l`: Move cursor right
-   Arrow Keys: Move cursor (Left, Down, Up, Right)
-   `i`: Enter **Insert Mode**
-   `:`: Enter **Command Mode**
-   `q`: Quit the editor (if no unsaved changes)

### 2. Insert Mode

In Insert Mode, you can type and modify the content of the file.

**Keybindings:**

-   `Esc`: Exit Insert Mode and return to **Normal Mode**
-   Typing characters: Inserts characters at the cursor position
-   `Enter`: Inserts a new line
-   `Backspace`: Deletes the character before the cursor
-   Arrow Keys: Move cursor (Left, Down, Up, Right)

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

## New File Creation

You can start `rim` without any arguments (e.g., just `cargo run`). This will open an empty buffer. You can then type your content and save it as a new file using the `:w <filename>` command.
