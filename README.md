# Polish - Rust Code Polishing Tool

A Rust script that automatically organizes, formats, and lints Rust code during git commits or on-demand. It groups declarations by visibility, sorts dependencies, and runs `cargo fmt` and `cargo clippy` only on affected workspace members.

## Features

### 🎯 Rust Declaration Grouping

Automatically organizes declarations at the top of Rust files with intelligent visibility-based grouping:

**Ordering:**
1. Global attributes (`#![feature(...)]`, `#![expect(...)]`, `#![warn(...)]`, `#![recursion_limit]`)
2. `extern crate` declarations
3. Module and use declarations grouped by visibility:
   - `pub` (most visible)
   - `pub(crate)`
   - `pub(super)`
   - `pub(in path)`
   - private (no visibility modifier)
4. Within each visibility level, `mod` declarations come before `use` statements

**Smart Comment Handling:**
- ✅ Preserves comments and attributes attached to declarations
- ✅ Comments with **no blank line** before code stay attached and move with the declaration
- ✅ Comments **separated by blank lines** from code are detached and stay in place
- ✅ Decorated items (with comments/attributes) are separated from undecorated items with blank lines
- ✅ Blank line separators are preserved to maintain code structure

**Additional Features:**
- ✅ Handles multi-line use statements
- ✅ Recursively processes nested modules
- ✅ Keeps `mod tests { ... }` blocks in place (not moved to top)
- ✅ Stops grouping at first non-declaration code (functions, structs, etc.)

### 📦 Cargo.toml Dependency Organization

Automatically organizes dependencies in `Cargo.toml`:
- **Groups dependencies** into two categories:
  1. External dependencies (crates.io)
  2. Workspace dependencies (using `path = "..."`)
- **Alphabetically sorts** within each group
- **Preserves comments** attached to dependencies
- **Handles multi-line** dependency definitions
- Applies to both `[dependencies]` and `[dev-dependencies]`

### ⚙️ Smart Cargo Integration

- Detects changed files in git commits
- Identifies affected workspace members
- Runs `cargo fmt` and `cargo clippy` only on affected packages
- Treats clippy warnings as errors (`-D warnings`)

## Prerequisites

- [rust-script](https://rust-script.org/) - Install with: `cargo install rust-script`
- Rust toolchain with `cargo fmt` and `cargo clippy`
- Git repository (when using git detection mode)

## Installation

1. Clone or download `polish.rs`
2. Make it executable:
   ```bash
   chmod +x polish.rs
   ```
3. Optionally add to PATH:
   ```bash
   # Add to ~/.bashrc or ~/.zshrc
   export PATH="$PATH:/path/to/polish"
   ```

## Usage

### Basic Usage (Git Mode)

Run on files changed in the last commit:
```bash
./polish.rs
```

This will:
1. Detect files changed in `HEAD~1`
2. Group Rust declarations and organize Cargo.toml dependencies
3. Run `cargo fmt` on affected packages
4. Run `cargo clippy` on affected packages

### Process Specific Files

Bypass git detection and process specific files:
```bash
./polish.rs --files src/main.rs Cargo.toml lib/utils.rs
```

### Skip Specific Operations

```bash
# Skip declaration grouping, only run fmt and clippy
./polish.rs --no-grouping

# Only group declarations, skip fmt and clippy
./polish.rs --no-fmt --no-clippy

# Only run grouping and fmt, skip clippy
./polish.rs --no-clippy

# Process specific files with only grouping
./polish.rs --files src/main.rs --no-fmt --no-clippy
```

### CLI Options

```
Options:
  --no-grouping       Skip grouping declarations and organizing dependencies
  --no-fmt            Skip running cargo fmt
  --no-clippy         Skip running cargo clippy
  --files <FILES>...  Process specific files (bypasses git detection)
  -h, --help          Print help
```

## Interactive Rebase Integration

### Manual Execution

During interactive rebase, add `exec` commands:
```bash
git rebase -i HEAD~15
```

In the editor:
```
pick abc123 feat: add new feature
exec /path/to/polish.rs
pick def456 fix: resolve bug
exec /path/to/polish.rs
```

### Automatic Execution

Create a git alias to automatically run polish on every commit:
```bash
git config alias.polish-rebase '!git rebase -i --exec "/path/to/polish.rs" HEAD~'
```

Usage:
```bash
git polish-rebase 15  # Polish last 15 commits
```

## Examples

### Example 1: Rust Declaration Grouping with Visibility

**Before:**
```rust
use std::collections::HashMap;
pub(crate) use internal::Helper;
pub use bar::baz;
mod inner;
pub mod api;
use foo::bar;

fn main() {
    println!("Hello");
}
```

**After:**
```rust
pub mod api;

pub use bar::baz;

pub(crate) use internal::Helper;

mod inner;

use std::collections::HashMap;
use foo::bar;

fn main() {
    println!("Hello");
}
```

### Example 2: Comment Attachment and Blank Line Preservation

**Before:**
```rust
// This comment is attached (no blank line)
use foo;

// This comment is detached (blank line before code)

use bar;
```

**After:**
```rust
// This comment is detached (blank line before code)

// This comment is attached (no blank line)
use foo;

use bar;
```

### Example 3: Decorated vs Undecorated Items

**Before:**
```rust
use std::fs;
#[cfg(test)]
use test_utils;
use std::io;
```

**After:**
```rust
#[cfg(test)]
use test_utils;

use std::fs;
use std::io;
```

### Example 4: Cargo.toml Dependencies

**Before:**
```toml
[dependencies]
tokio = "1.0"
serde = "1.0"
my_local_crate = { path = "../my_local_crate" }
anyhow = "1.0"
another_local = { path = "../another" }
```

**After:**
```toml
[dependencies]
anyhow = "1.0"
serde = "1.0"
tokio = "1.0"

another_local = { path = "../another" }
my_local_crate = { path = "../my_local_crate" }
```

## Architecture

The script is organized into modules:

- **Main**: CLI parsing, git integration, cargo operations
- **`rust_grouping`**: Rust declaration grouping logic with 33 tests
- **`toml_grouping`**: Cargo.toml dependency organization with 8 tests

**Total: 41 tests** covering edge cases like nested modules, decorated items, blank line preservation, and comment handling.

All parsing is done manually (no external dependencies beyond standard tooling).

## How It Works

### 1. File Detection
   - Git mode: Uses `git diff --name-only HEAD~1`
   - Files mode: Uses provided file paths

### 2. File Classification
   - Identifies Rust files (`.rs`)
   - Identifies Cargo.toml files

### 3. Rust Declaration Processing

   **State Machine Parser:**
   - **Header Mode**: Collects declarations (mod, use, extern crate, global attributes)
   - **Classification**: Each line is classified as Pending (comment, attribute, blank) or Item (declaration, code)
   - **Pending Lines**: Comments and attributes accumulate until an item is found
   - **Attachment Rules**:
     - Comments with no blank line before code → attached to that code
     - Comments with blank line before code → detached, stay in place
   - **Grouping**: Items grouped by visibility (pub → pub(crate) → pub(super) → pub(in) → private) then by kind (mod, use)
   - **Decorated Items**: Items with comments/attributes separated from undecorated items
   - **Exit Header Mode**: At first non-declaration (fn, struct, impl, etc.)
   - **Nested Modules**: Recursively processes `mod name { ... }` blocks

### 4. Cargo.toml Processing

   **Line-by-Line Parser:**
   - Identifies `[dependencies]` and `[dev-dependencies]` sections
   - Collects dependencies with their comments
   - Separates workspace (path) vs external dependencies
   - Sorts each group alphabetically
   - Outputs: external deps first, blank line, then workspace deps

### 5. Cargo Integration
   - Maps files to workspace members by walking up directory tree to find Cargo.toml
   - Runs `cargo fmt -p <member>` for each affected package
   - Runs `cargo clippy -p <member> --all-targets -- -D warnings`

## Key Behaviors

### Comment Attachment
- **Attached**: Comment with NO blank line before code moves with the code
- **Detached**: Comment with blank line before code stays in place

### Blank Line Preservation
- Blank lines between comments and code are preserved
- Trailing blank lines in comment blocks are kept to maintain separation
- Blank lines between decorated and undecorated items are maintained

### Visibility Ordering
Items are sorted by visibility (most visible first):
1. `pub` - public to all
2. `pub(crate)` - public within crate
3. `pub(super)` - public to parent module
4. `pub(in path)` - public within specific path
5. (no modifier) - private

## Testing

Run the test suite:
```bash
cargo test
```

Or with rust-script:
```bash
rust-script --test polish.rs
```

## License

MIT
