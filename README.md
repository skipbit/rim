# rim

`rim` is a terminal editor built in Rust.

## Project Goal

This project aims to eventually surpass Vim and NeoVim.

## Features (Planned)

- **Built with Rust**: Fast and safe performance.
- **Beyond NeoVim Experience**: Inheriting the benefits of existing editors while offering new functionalities.
- **AI Code Assist**: AI-powered development support including code completion, refactoring suggestions, and bug detection.

## Tech Stack

- **Rust**: Systems programming language

## Setup

### Prerequisites

- Rust must be installed.
  You can install it from the [official Rust website](https://www.rust-lang.org/tools/install).

### Clone the repository

```bash
git clone https://github.com/skipbit/rim.git
cd rim
```

## Build and Run

### Build

To build the project, run the following command:

```bash
cargo build
```

### Run

After building, you can run the editor with the following command:

```bash
cargo run -- <filename>
# Or
./target/debug/rim <filename>
```

Replace `<filename>` with the path to the file you want to open.

## Development

### Testing

To run tests, use the following command:

```bash
cargo test
```

### Linting

For code quality checks, `clippy` is used:

```bash
cargo clippy
```

### Format Check

To check code formatting, run the following command:

```bash
cargo fmt -- --check
```