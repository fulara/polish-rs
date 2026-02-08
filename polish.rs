#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! anyhow = "1.0"
//! clap = { version = "4.5", features = ["derive"] }
//! serde = { version = "1.0", features = ["derive"] }
//! serde_json = "1.0"
//! ```

use anyhow::{bail, Context};
use clap::Parser;
use std::collections::HashSet;
use std::ffi;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq)]
enum FileType {
    Rust,
    CargoToml,
}

#[derive(Parser, Debug)]
#[command(name = "polish-rs")]
#[command(about = "Format and lint Rust code in git repository", long_about = None)]
struct Cli {
    /// Skip grouping declarations
    #[arg(long)]
    no_grouping: bool,

    /// Skip running cargo fmt
    #[arg(long)]
    no_fmt: bool,

    /// Skip running cargo clippy
    #[arg(long)]
    no_clippy: bool,

    /// Process specific files instead of using git to detect changes
    #[arg(long, num_args = 1..)]
    files: Vec<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Check if we're in a git repository
    if !is_git_repo()? {
        bail!("Not in a git repository");
    }

    // Get the root of the git repository
    let git_root = get_git_root()?;
    println!("Git root: {}", git_root.display());

    // Get files to process
    let files_to_process = if cli.files.is_empty() {
        // Get changed files from git
        let changed = get_changed_files()?;
        if changed.is_empty() {
            println!("No files changed in current commit");
            return Ok(());
        }
        println!("Changed files: {:?}", changed);
        changed
    } else {
        // Use explicitly provided files
        println!("Processing specified files: {:?}", cli.files);
        classify_files(&cli.files)?
    };

    // Group declarations and organize dependencies
    if !cli.no_grouping {
        for (file_path, file_type) in &files_to_process {
            match file_type {
                FileType::Rust => {
                    rust_grouping::group_file_declarations(&file_path)?;
                }
                FileType::CargoToml => {
                    toml_grouping::organize_dependencies(&file_path)?;
                }
            }
        }
    }

    // Find affected workspace members (only for Rust files)
    let rust_files: Vec<PathBuf> = files_to_process
        .iter()
        .filter(|(_, ft)| *ft == FileType::Rust)
        .map(|(p, _)| p.clone())
        .collect();

    if rust_files.is_empty() {
        println!("No Rust files to format/lint");
        return Ok(());
    }

    let workspace_members = find_affected_projects(&git_root, &rust_files)?;

    if workspace_members.is_empty() {
        println!("No Rust workspace members affected");
        return Ok(());
    }

    println!("Affected workspace members: {:?}", workspace_members);

    // Run cargo fmt on all affected members in a single call
    if !cli.no_fmt {
        run_cargo_fmt(&git_root, &workspace_members)?;
    }

    // Run cargo clippy on all affected members in a single call
    if !cli.no_clippy {
        run_cargo_clippy(&git_root, &workspace_members)?;
    }

    println!("✓ All checks passed!");
    Ok(())
}

fn is_git_repo() -> anyhow::Result<bool> {
    let output = Command::new("git")
        .args(&["rev-parse", "--git-dir"])
        .output()?;
    Ok(output.status.success())
}

fn get_git_root() -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .args(&["rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to get git root")?;

    if !output.status.success() {
        bail!("Failed to get git root");
    }

    let path = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(path))
}

fn get_changed_files() -> anyhow::Result<Vec<(PathBuf, FileType)>> {
    // Get changed files (staged and unstaged), excluding deleted files
    // For renames, --name-only will show the new name
    let output = Command::new("git")
        .args(&["diff", "--diff-filter=d", "--name-only", "HEAD~1"])
        .output()
        .context("Failed to get changed files")?;

    if !output.status.success() {
        bail!("Failed to get changed files");
    }

    let paths: Vec<PathBuf> = String::from_utf8(output.stdout)?
        .lines()
        .map(|line| PathBuf::from(line.trim()))
        .filter(|path| !path.as_os_str().is_empty())
        .collect();

    classify_files(&paths)
}

fn classify_files(paths: &[PathBuf]) -> anyhow::Result<Vec<(PathBuf, FileType)>> {
    let mut result = Vec::new();

    for path in paths {
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if file_name == "Cargo.toml" {
                result.push((path.clone(), FileType::CargoToml));
            } else if path.extension() == Some(ffi::OsStr::new("rs")) {
                result.push((path.clone(), FileType::Rust));
            }
            // Ignore other file types
        }
    }

    Ok(result)
}

fn find_affected_projects(
    git_root: &Path,
    changed_files: &[PathBuf],
) -> anyhow::Result<HashSet<String>> {
    let mut affected_members = HashSet::new();

    for changed_file in changed_files {
        // Skip non-Rust files
        if changed_file.extension() != Some(ffi::OsStr::new("rs")) {
            continue;
        }

        // Find the package for this file by walking up the directory tree
        let package_name = find_project_for_file(git_root, changed_file)?;
        affected_members.insert(package_name);
    }

    Ok(affected_members)
}

fn find_project_for_file(git_root: &Path, file: &Path) -> anyhow::Result<String> {
    // Start from the file's directory
    let full_path = git_root.join(file);
    let mut current_dir = if full_path.is_file() {
        full_path.parent()
    } else {
        Some(full_path.as_path())
    };

    // Walk up the directory tree until we find a Cargo.toml or reach git root
    while let Some(dir) = current_dir {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            return Ok(dir
                .file_name()
                .context("dir file_name should work")?
                .to_string_lossy()
                .to_string());
        }
        anyhow::ensure!(dir != git_root, "Can't go beyond git's root directory");
        // Go up one directory
        current_dir = dir.parent();
    }
    anyhow::bail!("There are no Cargo.toml in the repo?")
}

fn run_cargo_fmt(git_root: &Path, members: &HashSet<String>) -> anyhow::Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("fmt");
    // Add -p flag for each member
    for member in members {
        cmd.arg("-p").arg(member);
    }
    let status = cmd
        .current_dir(git_root)
        .status()
        .context("Failed to run cargo fmt")?;
    println!("Running {cmd:?}");
    if !status.success() {
        bail!("cargo fmt failed");
    }
    Ok(())
}

fn run_cargo_clippy(git_root: &Path, members: &HashSet<String>) -> anyhow::Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.arg("clippy");
    // Add -p flag for each member
    for member in members {
        cmd.arg("-p").arg(member);
    }
    cmd.args(&["--all-targets", "--", "-D", "warnings"]);
    println!("Running {cmd:?}");
    let status = cmd
        .current_dir(git_root)
        .status()
        .context("Failed to run cargo clippy")?;

    if !status.success() {
        bail!("cargo clippy found warnings");
    }

    Ok(())
}

mod toml_grouping {
    use anyhow::Context;
    use std::fs;
    use std::path::Path;

    pub fn organize_dependencies(file_path: &Path) -> anyhow::Result<()> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

        let organized_content = organize_toml(&content)?;

        fs::write(file_path, organized_content)
            .with_context(|| format!("Failed to write file: {}", file_path.display()))?;

        Ok(())
    }

    fn organize_toml(content: &str) -> anyhow::Result<String> {
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let mut result = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let line = &lines[i];
            let trimmed = line.trim();

            if trimmed == "[dependencies]" || trimmed == "[dev-dependencies]" {
                // Found a dependencies section
                result.push(line.clone());
                i += 1;

                // Collect all dependencies in this section
                let (deps, next_idx) = collect_dependencies(&lines, i);

                // Organize and sort the dependencies
                let organized = organize_dependency_group(&deps);
                result.extend(organized);

                i = next_idx;
            } else {
                result.push(line.clone());
                i += 1;
            }
        }

        Ok(result.join("\n") + "\n")
    }

    fn collect_dependencies(lines: &[String], start: usize) -> (Vec<String>, usize) {
        let mut deps = Vec::new();
        let mut i = start;

        while i < lines.len() {
            let line = &lines[i];
            let trimmed = line.trim();

            // Stop at next section or empty line followed by section
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                break;
            }

            // Stop at end of file or double blank line
            if i > start && trimmed.is_empty() {
                // Check if next non-empty line is a section
                let mut lookahead = i + 1;
                while lookahead < lines.len() && lines[lookahead].trim().is_empty() {
                    lookahead += 1;
                }
                if lookahead < lines.len() {
                    let next_trimmed = lines[lookahead].trim();
                    if next_trimmed.starts_with('[') && next_trimmed.ends_with(']') {
                        break;
                    }
                }
            }

            deps.push(line.clone());
            i += 1;
        }

        (deps, i)
    }

    fn organize_dependency_group(deps: &[String]) -> Vec<String> {
        let mut workspace_deps = Vec::new();
        let mut external_deps = Vec::new();
        let mut current_dep = Vec::new();
        let mut pending_comments = Vec::new();
        let mut is_multiline = false;

        for line in deps {
            let trimmed = line.trim();

            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with('#') {
                // Comment line - accumulate for next dependency
                pending_comments.push(line.clone());
                continue;
            }

            // Check if this starts a new dependency (has '=')
            if !is_multiline && trimmed.contains('=') {
                // Finish previous dependency if any
                if !current_dep.is_empty() {
                    let dep_text = current_dep.join("\n");
                    if is_workspace_dep(&dep_text) {
                        workspace_deps.push(dep_text);
                    } else {
                        external_deps.push(dep_text);
                    }
                    current_dep.clear();
                }

                // Start new dependency with pending comments
                current_dep.extend(pending_comments.drain(..));
                current_dep.push(line.clone());

                // Check if this is a multiline dependency (ends with { but no })
                is_multiline = trimmed.contains('{') && !trimmed.contains('}');
            } else {
                // Continuation of current dependency
                current_dep.push(line.clone());
                if is_multiline && trimmed.contains('}') {
                    is_multiline = false;
                }
            }
        }

        // Add last dependency
        if !current_dep.is_empty() {
            let dep_text = current_dep.join("\n");
            if is_workspace_dep(&dep_text) {
                workspace_deps.push(dep_text);
            } else {
                external_deps.push(dep_text);
            }
        }

        // Sort each group
        workspace_deps.sort_by_key(|d| extract_dep_name(d).to_lowercase());
        external_deps.sort_by_key(|d| extract_dep_name(d).to_lowercase());

        // Combine groups with blank line separator
        let mut result = Vec::new();

        for dep in &workspace_deps {
            result.push(dep.clone());
        }

        if !workspace_deps.is_empty() && !external_deps.is_empty() {
            result.push(String::new()); // Blank line between groups
        }

        for dep in &external_deps {
            result.push(dep.clone());
        }

        result
    }

    fn is_workspace_dep(dep: &str) -> bool {
        dep.contains("path =") || dep.contains("path=")
    }

    fn extract_dep_name(dep: &str) -> String {
        // Extract dependency name from lines like: name = "version"
        for line in dep.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            if let Some(eq_pos) = trimmed.find('=') {
                return trimmed[..eq_pos].trim().to_string();
            }
        }
        String::new()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_basic_sorting() {
            let input = r#"[dependencies]
serde = "1.0"
anyhow = "1.0"
clap = "4.0"
"#;

            let expected = r#"[dependencies]
anyhow = "1.0"
clap = "4.0"
serde = "1.0"
"#;

            let result = organize_toml(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_workspace_deps_first() {
            let input = r#"[dependencies]
serde = "1.0"
my_local_crate = { path = "../my_local_crate" }
anyhow = "1.0"
other_local = { path = "../other" }
"#;

            let expected = r#"[dependencies]
my_local_crate = { path = "../my_local_crate" }
other_local = { path = "../other" }

anyhow = "1.0"
serde = "1.0"
"#;

            let result = organize_toml(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_dev_dependencies() {
            let input = r#"[dependencies]
serde = "1.0"

[dev-dependencies]
criterion = "0.5"
my_test_utils = { path = "../test_utils" }
"#;

            let expected = r#"[dependencies]
serde = "1.0"

[dev-dependencies]
my_test_utils = { path = "../test_utils" }

criterion = "0.5"
"#;

            let result = organize_toml(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_multiline_dependencies() {
            let input = r#"[dependencies]
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = [
    "derive",
    "rc",
] }
anyhow = "1.0"
"#;

            let expected = r#"[dependencies]
anyhow = "1.0"
serde = { version = "1.0", features = [
    "derive",
    "rc",
] }
tokio = { version = "1.0", features = ["full"] }
"#;

            let result = organize_toml(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_preserves_other_sections() {
            let input = r#"[package]
name = "test"
version = "0.1.0"

[dependencies]
serde = "1.0"
anyhow = "1.0"

[dev-dependencies]
criterion = "0.5"

[build-dependencies]
cc = "1.0"
"#;

            let expected = r#"[package]
name = "test"
version = "0.1.0"

[dependencies]
anyhow = "1.0"
serde = "1.0"

[dev-dependencies]
criterion = "0.5"

[build-dependencies]
cc = "1.0"
"#;

            let result = organize_toml(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_with_comments() {
            let input = r#"[dependencies]
# Serialization
serde = "1.0"
# Error handling
anyhow = "1.0"
# Local workspace crate
my_crate = { path = "../my_crate" }
"#;

            let expected = r#"[dependencies]
# Local workspace crate
my_crate = { path = "../my_crate" }

# Error handling
anyhow = "1.0"
# Serialization
serde = "1.0"
"#;

            let result = organize_toml(input).unwrap();
            assert_eq!(result, expected);
        }
    }
}

mod rust_grouping {
    use anyhow::Context;
    use std::fs;
    use std::path::Path;

    #[derive(Debug, Clone, PartialEq)]
    enum LineType {
        Blank,
        Comment,
        Feature,
        Use,
        PubUse,
        Mod,
        PubMod,
        Attribute,
        OtherCode,
    }

    #[derive(Debug)]
    struct Item {
        lines: Vec<String>,
        item_type: LineType,
    }

    pub fn group_file_declarations(file_path: &Path) -> anyhow::Result<()> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

        let grouped_content = group_items(&content)?;

        fs::write(file_path, grouped_content)
            .with_context(|| format!("Failed to write file: {}", file_path.display()))?;

        Ok(())
    }

    pub fn group_items(content: &str) -> anyhow::Result<String> {
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let mut result = String::new();
        let mut index = 0;

        process_scope(&lines, &mut index, &mut result, 0)?;

        Ok(result)
    }

    fn process_scope(
        lines: &[String],
        index: &mut usize,
        result: &mut String,
        indent_level: usize,
    ) -> anyhow::Result<()> {
        let mut features = Vec::new();
        let mut uses = Vec::new();
        let mut pub_uses = Vec::new();
        let mut mods = Vec::new();
        let mut pub_mods = Vec::new();

        let mut current_item_lines = Vec::new();
        let mut in_header = true;
        let mut pending_attributes = Vec::new();
        let mut has_items = false; // Track if we've added any items yet

        while *index < lines.len() {
            let line = &lines[*index];
            let trimmed = line.trim();

            // Check for end of module scope (closing brace at appropriate indent)
            if indent_level > 0 && trimmed == "}" {
                break;
            }

            let line_type = classify_line(trimmed);

            match line_type {
                LineType::Blank => {
                    if in_header {
                        // Only accumulate blank lines after we've seen at least one item
                        // This preserves blank lines within groups but not before first item
                        if has_items {
                            current_item_lines.push(line.clone());
                        }
                    } else {
                        result.push_str(line);
                        result.push('\n');
                    }
                    *index += 1;
                }
                LineType::Comment => {
                    if in_header {
                        current_item_lines.push(line.clone());
                    } else {
                        result.push_str(line);
                        result.push('\n');
                    }
                    *index += 1;
                }
                LineType::Attribute => {
                    if in_header {
                        pending_attributes.push(line.clone());
                    } else {
                        result.push_str(line);
                        result.push('\n');
                    }
                    *index += 1;
                }
                LineType::Feature
                | LineType::Use
                | LineType::PubUse
                | LineType::Mod
                | LineType::PubMod => {
                    if !in_header {
                        result.push_str(line);
                        result.push('\n');
                        *index += 1;
                        continue;
                    }

                    // Collect the complete statement
                    current_item_lines.append(&mut pending_attributes);

                    // Only keep non-blank lines from accumulated lines (comments/attributes)
                    // Discard blank lines as flush_groups will handle inter-group spacing
                    let non_blank_lines: Vec<String> = current_item_lines
                        .iter()
                        .filter(|l| !l.trim().is_empty())
                        .cloned()
                        .collect();
                    current_item_lines = non_blank_lines;

                    let (complete_item, next_index) =
                        collect_complete_item(lines, *index, &line_type)?;
                    current_item_lines.extend(complete_item);
                    *index = next_index;

                    // Check if this is a mod block (with body) - treat it like other code
                    if (line_type == LineType::Mod || line_type == LineType::PubMod)
                        && has_mod_block(&current_item_lines)
                    {
                        // Flush header groups before the mod block
                        flush_groups(result, &features, &uses, &pub_uses, &mods, &pub_mods);
                        in_header = false;

                        // Add blank line between groups and mod block if there were groups
                        if has_items {
                            result.push('\n');
                        }

                        // Output the mod declaration up to the opening brace
                        for item_line in &current_item_lines {
                            result.push_str(item_line);
                            result.push('\n');
                        }

                        // Process the content inside the mod block recursively
                        process_scope(lines, index, result, indent_level + 1)?;

                        // Add the closing brace
                        if *index < lines.len() {
                            result.push_str(&lines[*index]);
                            result.push('\n');
                            *index += 1;
                        }

                        current_item_lines.clear();
                    } else {
                        // Regular declaration (including mod foo; without body)
                        let item = Item {
                            lines: current_item_lines.clone(),
                            item_type: line_type,
                        };

                        match item.item_type {
                            LineType::Feature => features.push(item),
                            LineType::Use => uses.push(item),
                            LineType::PubUse => pub_uses.push(item),
                            LineType::Mod => mods.push(item),
                            LineType::PubMod => pub_mods.push(item),
                            _ => unreachable!(),
                        }
                        has_items = true;

                        current_item_lines.clear();
                    }
                }
                LineType::OtherCode => {
                    if in_header {
                        // Flush all groups when transitioning out of header
                        flush_groups(result, &features, &uses, &pub_uses, &mods, &pub_mods);
                        in_header = false;

                        // Output any pending lines (blank lines, comments)
                        for pending_line in &current_item_lines {
                            result.push_str(pending_line);
                            result.push('\n');
                        }
                        current_item_lines.clear();

                        // Output any pending attributes (after blank lines)
                        for pending_attr in &pending_attributes {
                            result.push_str(pending_attr);
                            result.push('\n');
                        }
                        pending_attributes.clear();
                    }

                    // Output the rest of the file as-is
                    result.push_str(line);
                    result.push('\n');
                    *index += 1;
                }
            }
        }

        // If we finished in header mode, flush groups
        if in_header {
            flush_groups(result, &features, &uses, &pub_uses, &mods, &pub_mods);
        }

        Ok(())
    }

    fn classify_line(trimmed: &str) -> LineType {
        if trimmed.is_empty() {
            return LineType::Blank;
        }

        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            return LineType::Comment;
        }

        if trimmed.starts_with("#![feature(") {
            return LineType::Feature;
        }

        if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
            return LineType::Attribute;
        }

        if trimmed.starts_with("pub use ")
            || trimmed.starts_with("pub(") && trimmed.contains(") use ")
        {
            return LineType::PubUse;
        }

        if trimmed.starts_with("use ") || trimmed.starts_with("extern crate ") {
            return LineType::Use;
        }

        if trimmed.starts_with("pub mod ")
            || trimmed.starts_with("pub(") && trimmed.contains(") mod ")
        {
            return LineType::PubMod;
        }

        if trimmed.starts_with("mod ") {
            return LineType::Mod;
        }

        LineType::OtherCode
    }

    fn collect_complete_item(
        lines: &[String],
        start_index: usize,
        item_type: &LineType,
    ) -> anyhow::Result<(Vec<String>, usize)> {
        let mut result = Vec::new();
        let mut index = start_index;

        // For mod blocks, we only collect until the opening brace
        if *item_type == LineType::Mod || *item_type == LineType::PubMod {
            while index < lines.len() {
                let line = lines[index].clone();
                result.push(line.clone());
                index += 1;

                let trimmed = line.trim();
                if trimmed.ends_with(';') {
                    // mod foo; declaration
                    break;
                }
                if trimmed.ends_with('{') {
                    // mod foo { ... } - stop at opening brace
                    break;
                }
            }
        } else if *item_type == LineType::Feature {
            // Features are complete on a single line ending with ']'
            while index < lines.len() {
                let line = lines[index].clone();
                result.push(line.clone());
                index += 1;

                let trimmed = line.trim();
                if trimmed.ends_with(']') {
                    break;
                }
            }
        } else {
            // For other items (use, pub use), collect until semicolon
            while index < lines.len() {
                let line = lines[index].clone();
                result.push(line.clone());
                index += 1;

                if line.trim().ends_with(';') {
                    break;
                }
            }
        }

        Ok((result, index))
    }

    fn has_mod_block(lines: &[String]) -> bool {
        for line in lines {
            let trimmed = line.trim();
            if trimmed.ends_with('{') {
                return true;
            }
            if trimmed.ends_with(';') {
                return false;
            }
        }
        false
    }

    fn flush_groups(
        result: &mut String,
        features: &[Item],
        uses: &[Item],
        pub_uses: &[Item],
        mods: &[Item],
        pub_mods: &[Item],
    ) {
        let groups = [features, pub_mods, pub_uses, mods, uses];
        let mut first_group = true;

        for group in &groups {
            if group.is_empty() {
                continue;
            }

            // Add blank line between groups (but not before first group)
            if !first_group {
                result.push('\n');
            }
            first_group = false;

            for item in *group {
                for line in &item.lines {
                    result.push_str(line);
                    result.push('\n');
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_basic_grouping() {
            let input = r#"mod inner;
use std::collections::HashMap;
pub use bar::baz;
use foo::bar;
pub mod tests;
"#;

            let expected = r#"pub mod tests;

pub use bar::baz;

mod inner;

use std::collections::HashMap;
use foo::bar;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_features_first() {
            let input = r#"use std::fs;
#![feature(test)]
#![feature(another)]
use std::io;
"#;

            let expected = r#"#![feature(test)]
#![feature(another)]

use std::fs;
use std::io;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_comments_attached() {
            let input = r#"use std::fs;
// Comment about HashMap
use std::collections::HashMap;
pub use bar::baz;
"#;

            let expected = r#"pub use bar::baz;

use std::fs;
// Comment about HashMap
use std::collections::HashMap;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_multiline_use() {
            let input = r#"use std::{
    collections::HashMap,
    fs,
};
pub use bar::baz;
"#;

            let expected = r#"pub use bar::baz;

use std::{
    collections::HashMap,
    fs,
};
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_attributes_attached() {
            let input = r#"use std::fs;
#[cfg(test)]
use test_utils;
pub use bar::baz;
"#;

            let expected = r#"pub use bar::baz;

use std::fs;
#[cfg(test)]
use test_utils;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_stops_at_other_code() {
            let input = r#"use std::fs;
pub use bar::baz;

fn main() {
    println!("Hello");
}

use should::not::move;
"#;

            let expected = r#"pub use bar::baz;

use std::fs;

fn main() {
    println!("Hello");
}

use should::not::move;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_nested_module() {
            let input = r#"use std::fs;

mod nested {
    use super::*;
    pub use other::thing;

    fn nested_fn() {}
}

fn main() {}
"#;

            let expected = r#"use std::fs;

mod nested {
    pub use other::thing;

    use super::*;

    fn nested_fn() {}
}

fn main() {}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_mod_declaration_vs_block() {
            let input = r#"mod foo;
use std::fs;
mod bar {
    use std::io;
}
"#;

            let expected = r#"mod foo;

use std::fs;

mod bar {
    use std::io;
}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_pub_crate_visibility() {
            let input = r#"use std::fs;
pub(crate) use internal::foo;
pub use external::bar;
pub(crate) mod internal_mod;
pub mod public_mod;
"#;

            let expected = r#"pub(crate) mod internal_mod;
pub mod public_mod;

pub(crate) use internal::foo;
pub use external::bar;

use std::fs;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_extern_crate() {
            let input = r#"extern crate serde;
use std::fs;
pub use bar::baz;
"#;

            let expected = r#"pub use bar::baz;

extern crate serde;
use std::fs;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_blank_lines_between_groups() {
            let input = r#"use std::fs;

use std::io;
pub use bar::baz;
"#;

            let expected = r#"pub use bar::baz;

use std::fs;
use std::io;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_complex_nested_modules() {
            let input = r#"use std::fs;

mod outer {
    use super::*;
    pub use other::thing;

    mod inner {
        use std::fmt;
        pub mod deep;

        fn inner_fn() {}
    }

    fn outer_fn() {}
}
"#;

            let expected = r#"use std::fs;

mod outer {
    pub use other::thing;

    use super::*;

    mod inner {
        pub mod deep;

        use std::fmt;

        fn inner_fn() {}
    }

    fn outer_fn() {}
}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_block_comments() {
            let input = r#"use std::fs;
/* This is a
   multi-line comment */
use std::io;
"#;

            let expected = r#"use std::fs;
/* This is a
   multi-line comment */
use std::io;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_mod_tests_at_end() {
            let input = r#"use std::collections::HashMap;
pub use bar::baz;

fn main() {
    println!("Hello");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {}
}
"#;

            let expected = r#"pub use bar::baz;

use std::collections::HashMap;

fn main() {
    println!("Hello");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_something() {}
}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_attribute_before_function() {
            let input = r#"#[expect(dead_code)]
fn foo_test() {}
"#;

            let expected = r#"#[expect(dead_code)]
fn foo_test() {}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_attributes_with_imports_and_function() {
            let input = r#"use std::fs;

#[expect(dead_code)]
fn foo_test() {}
"#;

            let expected = r#"use std::fs;

#[expect(dead_code)]
fn foo_test() {}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }
    }
}
