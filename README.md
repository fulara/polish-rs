# Polish - Rust Code Polishing Tool

A Rust script that automatically organizes, formats, and lints Rust code during git commits or on-demand. It groups declarations, sorts dependencies, runs `cargo fmt` and `cargo clippy` only on affected workspace members.

## Features

### 🎯 Rust Declaration Grouping
Automatically organizes declarations at the top of Rust files in this order:
1. `#![feature(...)]` attributes
2. `pub mod` declarations
3. `pub use` statements
4. `mod` declarations
5. `pub use` statements
5. `use` statements

- ✅ Preserves comments and attributes with their declarations
- ✅ Handles multi-line use statements
- ✅ Recursively processes nested modules
- ✅ Keeps `mod tests { ... }` blocks in place (not moved to top)
- ✅ Stops at first non-declaration code (functions, structs, etc.)

### 📦 Cargo.toml Dependency Organization
Automatically organizes dependencies in `Cargo.toml`:
- **Groups dependencies** into two categories:
  1. Workspace dependencies (using `path = "..."`)
  2. External dependencies
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

### Before & After: Rust Declaration Grouping

**Before:**
```rust
mod tests;
use std::collections::HashMap;
pub use bar::baz;
use foo::bar;
pub mod api;

fn main() {
    println!("Hello");
}

mod inner {
    use super::*;
    pub use nested::Thing;
}
```

**After:**
```rust
pub mod api;

pub use bar::baz;

mod tests;

use std::collections::HashMap;
use foo::bar;

fn main() {
    println!("Hello");
}

mod inner {
    pub use nested::Thing;

    use super::*;
}
```

### Before & After: Cargo.toml Dependencies

**Before:**
```toml
[dependencies]
tokio="1.0"
serde = "1.0"
my_local_crate = { path = "../my_local_crate" }
anyhow= "1.0"
another_local= {path="../another"}
```

**After:**
```toml
[dependencies]
another_local = { path = "../another" }
my_local_crate = { path = "../my_local_crate" }

anyhow = "1.0"
serde = "1.0"
tokio = "1.0"
```

## Architecture

The script is organized into modules:

- **Main**: CLI parsing, git integration, cargo operations
- **`rust_grouping`**: Rust declaration grouping logic with 16 tests
- **`toml_grouping`**: Cargo.toml dependency organization with 9 tests

All parsing is done manually (no external dependencies beyond standard tooling).

## How It Works

1. **File Detection**:
   - Git mode: Uses `git diff --name-only HEAD~1`
   - Files mode: Uses provided file paths

2. **File Classification**:
   - Identifies Rust files (`.rs`)
   - Identifies Cargo.toml files

3. **Processing**:
   - Rust files: Groups declarations using state machine parser
   - Cargo.toml: Organizes dependencies by parsing line-by-line

4. **Cargo Integration**:
   - Maps files to workspace members
   - Runs `cargo fmt -p <member>` for each affected package
   - Runs `cargo clippy -p <member> --all-targets -- -D warnings`

## Testing

Run the test suite:
```bash
rust-script --test polish.rs
```

## License

MIT
