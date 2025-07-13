# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

Build the project:
```bash
cargo build
```

Run the editor:
```bash
cargo run -- <filename>
# Or without a file to start with empty buffer
cargo run
```

Run tests:
```bash
cargo test
```

Check code quality:
```bash
cargo clippy
```

Format code:
```bash
cargo fmt
```

Check formatting without applying:
```bash
cargo fmt -- --check
```

## Architecture Overview

rim follows a clean layered architecture with three main layers:

### Domain Layer (`src/domain/`)
- **`editor_model.rs`**: Core editor state and business logic
  - `EditorModel` struct holds all editor state (lines, cursor position, mode, history)
  - `EditorMode` enum defines Normal/Insert/Command/Search modes
  - `LastChange` enum tracks changes for the repeat (.) command
  - Implements undo/redo with full state snapshots

### Application Layer (`src/application/`)
- **`editor_service.rs`**: Main service orchestrating editor operations
  - `EditorService<T: FileIO>` provides high-level operations
  - Bridges between domain model and infrastructure
  - Handles command execution and returns `HandleCommandResult`
  
- **`commands.rs`**: Command pattern for ex-commands (:w, :q, :e)
  - `EditorCommand` trait for polymorphic command execution
  - Each command (Write, Quit, Edit) implements the trait

- **`normal_commands.rs`**: Command pattern for normal mode operations
  - `NormalCommand` trait for all normal mode key bindings
  - Implements vim-like commands (movement, editing, operators)
  - `DKeyHandler` implements the `d` prefix commands (dd for delete line)

### Infrastructure Layer (`src/infrastructure/`)
- **`file_io.rs`**: File system abstraction
  - `FileIO` trait enables testing with mock implementations
  - `LocalFileIO` provides actual file system operations

- **`terminal_ui.rs`**: Terminal rendering
  - `draw_editor()` handles all screen rendering
  - Uses crossterm for terminal manipulation
  - Displays mode indicators, cursor position, and status messages

### Main Entry Point (`src/main.rs`)
- Sets up terminal in raw mode with alternate screen
- Creates HashMap of normal mode commands for efficient key binding lookup
- Main event loop handles different modes with pattern matching
- Properly cleans up terminal on exit

## Key Design Patterns

1. **Command Pattern**: Both normal mode operations and ex-commands use command pattern for extensibility
2. **Dependency Injection**: `FileIO` trait allows testing without actual file system
3. **State Pattern**: Different editor modes have different key handling behavior
4. **History Pattern**: Undo/redo implemented with state snapshots

## Adding New Features

When implementing new vim commands:
1. For normal mode: Create a struct implementing `NormalCommand` in `normal_commands.rs`
2. For ex-commands: Create a struct implementing `EditorCommand` in `commands.rs`
3. Register the command in the appropriate HashMap/handler in `main.rs`
4. Update the domain model if new state is needed