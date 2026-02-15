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
                    rust_grouping::group_file_declarations(file_path)?;
                }
                FileType::CargoToml => {
                    toml_grouping::organize_dependencies(file_path)?;
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
        .args(["rev-parse", "--git-dir"])
        .output()?;
    Ok(output.status.success())
}

fn get_git_root() -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
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
        .args(["diff", "--diff-filter=d", "--name-only", "HEAD~1"])
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
            // Parse Cargo.toml to extract the package name
            let content = std::fs::read_to_string(&cargo_toml)
                .with_context(|| format!("Failed to read {}", cargo_toml.display()))?;
            return extract_package_name(&content);
        }
        anyhow::ensure!(dir != git_root, "Can't go beyond git's root directory");
        // Go up one directory
        current_dir = dir.parent();
    }
    anyhow::bail!("There are no Cargo.toml in the repo?")
}

fn extract_package_name(toml_content: &str) -> anyhow::Result<String> {
    let mut in_package_section = false;

    for line in toml_content.lines() {
        let trimmed = line.trim();

        if trimmed == "[package]" {
            in_package_section = true;
            continue;
        }

        if in_package_section {
            // Stop at next section
            if trimmed.starts_with('[') {
                break;
            }

            // Split on = and check if we have exactly 2 parts
            let parts: Vec<&str> = trimmed.split('=').collect();
            if parts.len() == 2 {
                let key = parts[0].trim();
                if key == "name" {
                    let value = parts[1].trim();
                    // Remove quotes
                    let name = value.trim_matches('"').trim_matches('\'');
                    return Ok(name.to_string());
                }
            }
        }
    }

    anyhow::bail!("Could not find package name in Cargo.toml")
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
    cmd.args(["--all-targets", "--", "-D", "warnings"]);
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
                current_dep.append(&mut pending_comments);
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

        // Combine groups with blank line separator - external deps first
        let mut result = Vec::new();

        for dep in &external_deps {
            result.push(dep.clone());
        }

        if !workspace_deps.is_empty() && !external_deps.is_empty() {
            result.push(String::new()); // Blank line between groups
        }

        for dep in &workspace_deps {
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
        fn test_external_deps_first() {
            let input = r#"[dependencies]
serde = "1.0"
my_local_crate = { path = "../my_local_crate" }
anyhow = "1.0"
other_local = { path = "../other" }
"#;

            let expected = r#"[dependencies]
anyhow = "1.0"
serde = "1.0"

my_local_crate = { path = "../my_local_crate" }
other_local = { path = "../other" }
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
criterion = "0.5"

my_test_utils = { path = "../test_utils" }
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
# Error handling
anyhow = "1.0"
# Serialization
serde = "1.0"

# Local workspace crate
my_crate = { path = "../my_crate" }
"#;

            let result = organize_toml(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_empty_braces_without_path() {
            let input = r#"[dependencies]
serde = "1.0"
tokio = { version = "1.0", features = ["full"] }
my_local = { path = "../local" }
anyhow = {}
clap = { version = "4.0" }
"#;

            let expected = r#"[dependencies]
anyhow = {}
clap = { version = "4.0" }
serde = "1.0"
tokio = { version = "1.0", features = ["full"] }

my_local = { path = "../local" }
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

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    enum Visibility {
        Pub, // Most visible
        PubCrate,
        PubSuper,
        PubIn(String), // Stores the path
        Private,       // Least visible
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    enum DeclarationKind {
        Mod,
        Use,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Declaration {
        Mod(Visibility),
        Use(Visibility),
    }

    impl Declaration {
        fn kind(&self) -> DeclarationKind {
            match self {
                Declaration::Mod(_) => DeclarationKind::Mod,
                Declaration::Use(_) => DeclarationKind::Use,
            }
        }

        fn visibility(&self) -> Visibility {
            match self {
                Declaration::Mod(v) => v.clone(),
                Declaration::Use(v) => v.clone(),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    enum GlobalAttribute {
        Feature,
        Expect,
        Warn,
        RecursionLimit,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum LineType {
        GlobalAttribute(GlobalAttribute),
        ExternCrate,
        Declaration(Declaration),
        OtherCode,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum LineClassification {
        Item(LineType),
        Pending,
    }

    #[derive(Debug, Clone)]
    struct Item {
        lines: Vec<String>,
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
        // Handle global attributes at the very beginning of the file
        // (expect, warn, recursion_limit, feature)
        if indent_level == 0 && *index < lines.len() {
            let first_line = lines[*index].trim();
            if first_line.starts_with("#![") {
                // Output global attributes at the beginning as-is
                while *index < lines.len() {
                    let line = &lines[*index];
                    let trimmed = line.trim();
                    let classification = classify_line(trimmed);

                    match classification {
                        LineClassification::Item(LineType::GlobalAttribute(_)) => {
                            result.push_str(line);
                            result.push('\n');
                            *index += 1;
                        }
                        LineClassification::Pending if trimmed.is_empty() => {
                            result.push_str(line);
                            result.push('\n');
                            *index += 1;
                        }
                        _ => break,
                    }
                }
            }
        }

        let mut features = Vec::new();
        let mut extern_crates = Vec::new();
        let mut declarations: std::collections::BTreeMap<
            Visibility,
            std::collections::BTreeMap<DeclarationKind, Vec<Item>>,
        > = std::collections::BTreeMap::new();

        let mut pending_lines = Vec::new(); // Accumulate attributes, comments, blank lines
        let mut in_header = true;
        let mut has_items = false; // Track if we've added any items yet
        let mut post_features_lines = Vec::new(); // Lines after global attributes
        let mut features_done = false; // Track if we've finished collecting global attributes

        while *index < lines.len() {
            let line = &lines[*index];
            let trimmed = line.trim();

            // Check for end of module scope (closing brace at appropriate indent)
            if indent_level > 0 && trimmed == "}" {
                break;
            }

            let classification = classify_line(trimmed);

            match classification {
                LineClassification::Pending => {
                    if in_header {
                        pending_lines.push(line.clone());
                    } else {
                        result.push_str(line);
                        result.push('\n');
                    }
                    *index += 1;
                }
                LineClassification::Item(item_type) => {
                    if !in_header {
                        result.push_str(line);
                        result.push('\n');
                        *index += 1;
                        continue;
                    }

                    // Check if this is first non-global-attribute (transition point)
                    let is_global_attr = matches!(item_type, LineType::GlobalAttribute(_));
                    if !features_done && !is_global_attr {
                        features_done = true;

                        // Check if pending_lines ends with blank lines (indicating separation)
                        let has_trailing_blanks = pending_lines
                            .iter()
                            .rev()
                            .take_while(|l| l.trim().is_empty())
                            .count()
                            > 0;

                        if has_trailing_blanks {
                            // Save everything except leading and trailing blanks as post_features_lines
                            let mut trailing_blank_count = 0;
                            for l in pending_lines.iter().rev() {
                                if l.trim().is_empty() {
                                    trailing_blank_count += 1;
                                } else {
                                    break;
                                }
                            }

                            let split_point = pending_lines.len() - trailing_blank_count;
                            let mut temp: Vec<String> =
                                pending_lines.drain(..split_point).collect();

                            // Strip leading blank lines
                            while !temp.is_empty() && temp[0].trim().is_empty() {
                                temp.remove(0);
                            }

                            post_features_lines = temp;
                            // At this point, pending_lines contains only trailing blank lines
                            // Keep one blank line to preserve separation from the following item
                            if pending_lines.len() > 1 {
                                pending_lines.drain(1..);
                            }
                        }
                    }

                    // Collect the complete statement with pending lines
                    let mut item_lines = pending_lines.clone();
                    pending_lines.clear();

                    // Collect the actual item (the declaration/attribute lines)
                    let (complete_item, next_index) =
                        collect_complete_item(lines, *index, &item_type)?;
                    item_lines.extend(complete_item);
                    *index = next_index;

                    // Handle based on item type
                    match item_type {
                        LineType::OtherCode => {
                            // Flush all groups when transitioning out of header
                            flush_groups(
                                result,
                                &features,
                                &post_features_lines,
                                &extern_crates,
                                &declarations,
                            );
                            in_header = false;

                            // Output any pending lines (attributes, comments, blanks)
                            for line in &item_lines {
                                result.push_str(line);
                                result.push('\n');
                            }
                        }
                        LineType::Declaration(Declaration::Mod(_))
                            if has_mod_block(&item_lines) =>
                        {
                            // Mod block with body - flush groups and process recursively
                            flush_groups(
                                result,
                                &features,
                                &post_features_lines,
                                &extern_crates,
                                &declarations,
                            );
                            in_header = false;

                            if has_items {
                                result.push('\n');
                            }

                            // Skip leading blank lines in item_lines (we already added one above)
                            for line in item_lines.iter().skip_while(|l| l.trim().is_empty()) {
                                result.push_str(line);
                                result.push('\n');
                            }

                            process_scope(lines, index, result, indent_level + 1)?;

                            if *index < lines.len() {
                                result.push_str(&lines[*index]);
                                result.push('\n');
                                *index += 1;
                            }
                        }
                        LineType::GlobalAttribute(_) => {
                            features.push(Item { lines: item_lines });
                            has_items = true;
                        }
                        LineType::ExternCrate => {
                            extern_crates.push(Item { lines: item_lines });
                            has_items = true;
                        }
                        LineType::Declaration(ref decl) => {
                            let kind = decl.kind();
                            let visibility = decl.visibility();
                            declarations
                                .entry(visibility)
                                .or_default()
                                .entry(kind)
                                .or_default()
                                .push(Item { lines: item_lines });
                            has_items = true;
                        }
                    }
                }
            }
        }

        // If we finished in header mode, flush groups
        if in_header {
            flush_groups(
                result,
                &features,
                &post_features_lines,
                &extern_crates,
                &declarations,
            );

            // Output any remaining pending lines (e.g., comments-only file)
            for line in &pending_lines {
                result.push_str(line);
                result.push('\n');
            }
        }

        Ok(())
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
        post_features_lines: &[String],
        extern_crates: &[Item],
        declarations: &std::collections::BTreeMap<
            Visibility,
            std::collections::BTreeMap<DeclarationKind, Vec<Item>>,
        >,
    ) {
        // Helper to check if an item is decorated (has comments or attributes)
        fn is_decorated(item: &Item) -> bool {
            // Check if any line before the actual item line is a comment or attribute
            for line in &item.lines {
                let trimmed = line.trim();
                if trimmed.starts_with("//")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with("#[")
                {
                    return true;
                }
                // Stop when we hit the actual item (not blank, not comment, not attribute)
                if !trimmed.is_empty()
                    && !trimmed.starts_with("//")
                    && !trimmed.starts_with("/*")
                    && !trimmed.starts_with('*')
                    && !trimmed.starts_with("#[")
                {
                    break;
                }
            }
            false
        }

        // Helper to output a group with decorated items first, then regular items
        fn output_group(result: &mut String, items: &[Item], first_group: &mut bool) {
            if items.is_empty() {
                return;
            }

            let mut decorated = Vec::new();
            let mut regular = Vec::new();

            for item in items {
                if is_decorated(item) {
                    decorated.push(item);
                } else {
                    regular.push(item);
                }
            }

            // Output decorated items first, each separated by whitespace
            for item in &decorated {
                // Add blank line between groups
                if !*first_group {
                    result.push('\n');
                }
                *first_group = false;

                // Skip leading blank lines to avoid double spacing
                for line in item.lines.iter().skip_while(|l| l.trim().is_empty()) {
                    result.push_str(line);
                    result.push('\n');
                }
            }

            // Output regular items together
            if !regular.is_empty() {
                // Add blank line between decorated and regular, or between groups
                if !*first_group {
                    result.push('\n');
                }
                *first_group = false;

                for item in &regular {
                    // Skip leading blank lines for regular items (they group together)
                    for line in item.lines.iter().skip_while(|l| l.trim().is_empty()) {
                        result.push_str(line);
                        result.push('\n');
                    }
                }
            }
        }

        let mut first_group = true;

        // Features (and related global attributes) go first - no splitting needed
        if !features.is_empty() {
            for item in features {
                for line in &item.lines {
                    result.push_str(line);
                    result.push('\n');
                }
            }
            first_group = false;
        }

        // Post-features comments/lines go after features but before other groups
        if !post_features_lines.is_empty() {
            if !first_group {
                result.push('\n');
            }
            for line in post_features_lines {
                result.push_str(line);
                result.push('\n');
            }
            first_group = false;
        }

        // Extern crates always come first (after features/post_features_lines)
        if !extern_crates.is_empty() {
            output_group(result, extern_crates, &mut first_group);
        }

        // Output declarations in BTreeMap order (automatically sorted)
        // Outer map: different Visibility (Pub, PubCrate, PubSuper, PubIn, Private)
        // Inner map: different DeclarationKind within same visibility (Mod, Use)
        for kind_map in declarations.values() {
            // Output each declaration kind within this visibility level
            for items in kind_map.values() {
                output_group(result, items, &mut first_group);
            }
        }
    }

    fn parse_visibility(decl_start: &str) -> Visibility {
        if decl_start.starts_with("pub(crate)") {
            Visibility::PubCrate
        } else if decl_start.starts_with("pub(super)") {
            Visibility::PubSuper
        } else if decl_start.starts_with("pub(in ") {
            // Extract path from pub(in path)
            if let Some(end) = decl_start.find(')') {
                let path = &decl_start[7..end]; // Skip "pub(in "
                Visibility::PubIn(path.trim().to_string())
            } else {
                Visibility::Pub // Fallback
            }
        } else if decl_start.starts_with("pub(") {
            // Other pub(...) variants we don't recognize
            Visibility::Pub
        } else if decl_start.starts_with("pub ") {
            Visibility::Pub
        } else {
            Visibility::Private
        }
    }

    fn classify_line(trimmed: &str) -> LineClassification {
        // Pending: things that should be accumulated (attributes, comments, blanks)
        if trimmed.is_empty() {
            return LineClassification::Pending;
        }

        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            return LineClassification::Pending;
        }

        if trimmed.starts_with("#[") {
            // Item attributes like #[cfg(test)], not global attributes
            return LineClassification::Pending;
        }

        // Global attributes
        if trimmed.starts_with("#![feature(") {
            return LineClassification::Item(LineType::GlobalAttribute(GlobalAttribute::Feature));
        }

        if trimmed.starts_with("#![expect(") {
            return LineClassification::Item(LineType::GlobalAttribute(GlobalAttribute::Expect));
        }

        if trimmed.starts_with("#![warn(") {
            return LineClassification::Item(LineType::GlobalAttribute(GlobalAttribute::Warn));
        }

        if trimmed.starts_with("#![recursion_limit") {
            return LineClassification::Item(LineType::GlobalAttribute(
                GlobalAttribute::RecursionLimit,
            ));
        }

        // Declarations
        if trimmed.starts_with("extern crate ") {
            return LineClassification::Item(LineType::ExternCrate);
        }

        // Check for use statements
        if trimmed.contains(" use ") || trimmed.starts_with("use ") {
            let visibility = parse_visibility(trimmed);
            return LineClassification::Item(LineType::Declaration(Declaration::Use(visibility)));
        }

        // Check for mod declarations
        if trimmed.contains(" mod ") || trimmed.starts_with("mod ") {
            let visibility = parse_visibility(trimmed);
            return LineClassification::Item(LineType::Declaration(Declaration::Mod(visibility)));
        }

        LineClassification::Item(LineType::OtherCode)
    }

    fn collect_complete_item(
        lines: &[String],
        start_index: usize,
        item_type: &LineType,
    ) -> anyhow::Result<(Vec<String>, usize)> {
        let mut result: Vec<String> = Vec::new();
        let mut index = start_index;

        match item_type {
            LineType::Declaration(Declaration::Mod(_)) => {
                // For mod blocks, we only collect until the opening brace or semicolon
                while index < lines.len() {
                    let line = lines[index].clone();
                    result.push(line.clone());
                    index += 1;

                    let trimmed = line.trim();
                    if trimmed.ends_with(';') || trimmed.ends_with('{') {
                        break;
                    }
                }
            }
            LineType::GlobalAttribute(_) => {
                // Global attributes are complete on a single line ending with ']'
                while index < lines.len() {
                    let line = lines[index].clone();
                    result.push(line.clone());
                    index += 1;

                    let trimmed = line.trim();
                    if trimmed.ends_with(']') {
                        break;
                    }
                }
            }
            _ => {
                // For other items (use, pub use, pub(crate) use, extern crate), collect until semicolon
                while index < lines.len() {
                    let line = lines[index].clone();
                    result.push(line.clone());
                    index += 1;

                    if line.trim().ends_with(';') {
                        break;
                    }
                }
            }
        }

        Ok((result, index))
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

// Comment about HashMap
use std::collections::HashMap;

use std::fs;
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

#[cfg(test)]
use test_utils;

use std::fs;
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

            let expected = r#"pub mod public_mod;

pub use external::bar;

pub(crate) mod internal_mod;

pub(crate) use internal::foo;

use std::fs;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_expect_at_beginning() {
            let input = r#"#![expect(dead_code)]
#![expect(unused_variables)]

use std::fs;
pub use bar::baz;

fn main() {}
"#;

            let expected = r#"#![expect(dead_code)]
#![expect(unused_variables)]

pub use bar::baz;

use std::fs;

fn main() {}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_comment_with_blank_line() {
            let input = r#"use std::fs;
// This is a comment about HashMap

use std::collections::HashMap;
pub use bar::baz;
"#;

            let expected = r#"pub use bar::baz;

// This is a comment about HashMap

use std::collections::HashMap;

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

            let expected = r#"extern crate serde;

pub use bar::baz;

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
        fn test_global_attributes_together() {
            let input = r#"#![feature(test)]
#![expect(dead_code)]
#![warn(unused_imports)]
#![recursion_limit = "256"]

use std::fs;
extern crate serde;
pub use bar::baz;
"#;

            let expected = r#"#![feature(test)]
#![expect(dead_code)]
#![warn(unused_imports)]
#![recursion_limit = "256"]

extern crate serde;

pub use bar::baz;

use std::fs;
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

        #[test]
        fn test_decorated_mods_separated() {
            let input = r#"mod foo;
#[macro_use]
mod types_into;
mod bar;
// Comment about baz
mod baz;
"#;

            let expected = r#"#[macro_use]
mod types_into;

// Comment about baz
mod baz;

mod foo;
mod bar;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_decorated_uses_separated() {
            let input = r#"use std::fs;
#[cfg(test)]
use test_utils;
use std::io;
// For HashMap
use std::collections::HashMap;
"#;

            let expected = r#"#[cfg(test)]
use test_utils;

// For HashMap
use std::collections::HashMap;

use std::fs;
use std::io;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_decorated_pub_uses_separated() {
            let input = r#"pub use foo::bar;
#[doc(hidden)]
pub use internal::secret;
pub use baz::qux;
"#;

            let expected = r#"#[doc(hidden)]
pub use internal::secret;

pub use foo::bar;
pub use baz::qux;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_comprehensive_ordering() {
            let input = r#"#![feature(test)]
#![expect(dead_code)]
#![warn(unused_imports)]
#![recursion_limit = "256"]

use std::io;
extern crate serde;
// This is a helper module
mod helper;
#[macro_use]
mod macros;
pub use bar::baz;
mod foo;
use std::fs;

fn main() {}
"#;

            let expected = r#"#![feature(test)]
#![expect(dead_code)]
#![warn(unused_imports)]
#![recursion_limit = "256"]

extern crate serde;

pub use bar::baz;

// This is a helper module
mod helper;

#[macro_use]
mod macros;

mod foo;

use std::io;
use std::fs;

fn main() {}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_multiple_decorated_and_undecorated() {
            let input = r#"pub mod alpha;
#[cfg(feature = "beta")]
pub mod beta;
pub mod gamma;
// Documentation for delta
pub mod delta;
pub mod epsilon;
"#;

            let expected = r#"#[cfg(feature = "beta")]
pub mod beta;

// Documentation for delta
pub mod delta;

pub mod alpha;
pub mod gamma;
pub mod epsilon;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_extern_crate_with_attributes() {
            let input = r#"extern crate foo;
#[macro_use]
extern crate serde;
extern crate bar;
use std::fs;
"#;

            let expected = r#"#[macro_use]
extern crate serde;

extern crate foo;
extern crate bar;

use std::fs;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_omega_all_constructs() {
            let input = r#"// line1: this is test comment to test grouping
// line2: this is test comment to test grouping
#![feature(test)]
#![feature(proc_macro_hygiene)]
#![expect(clippy::all)]
#![warn(rust_2018_idioms)]
#![warn(missing_docs)]
#![recursion_limit = "512"]

// line3: this is test comment to test grouping
// line4: this is test comment to test grouping

use std::io;
extern crate libc;
#[macro_use]
extern crate serde;
// This is a regular comment
extern crate log;
// Comment with blank line after

extern crate regex;
pub mod api;
#[cfg(feature = "server")]
pub mod server;
// Public client module
pub mod client;
pub mod utils;
pub use exported::Thing;
#[doc(hidden)]
pub use internal::Secret;
// This is a public export
pub use another::Export;
pub(crate) use internal::Helper;
// Comment about crate-visible import
pub(crate) use shared::Data;
mod parser;
#[macro_use]
mod macros;
// Private helper
mod helper;
mod config;
use std::collections::HashMap;
// Import with comment
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("Hello, world!");
}
"#;

            let expected = r#"// line1: this is test comment to test grouping
// line2: this is test comment to test grouping
#![feature(test)]
#![feature(proc_macro_hygiene)]
#![expect(clippy::all)]
#![warn(rust_2018_idioms)]
#![warn(missing_docs)]
#![recursion_limit = "512"]

// line3: this is test comment to test grouping
// line4: this is test comment to test grouping

#[macro_use]
extern crate serde;

// This is a regular comment
extern crate log;

// Comment with blank line after

extern crate regex;

extern crate libc;

#[cfg(feature = "server")]
pub mod server;

// Public client module
pub mod client;

pub mod api;
pub mod utils;

#[doc(hidden)]
pub use internal::Secret;

// This is a public export
pub use another::Export;

pub use exported::Thing;

// Comment about crate-visible import
pub(crate) use shared::Data;

pub(crate) use internal::Helper;

#[macro_use]
mod macros;

// Private helper
mod helper;

mod parser;
mod config;

// Import with comment
use std::fs;

use std::io;
use std::collections::HashMap;
use std::path::PathBuf;

fn main() {
    println!("Hello, world!");
}
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_pub_super_visibility() {
            let input = r#"use std::fs;
pub(super) use parent::foo;
pub use external::bar;
pub(in crate::utils) use internal::baz;
pub(crate) mod internal_mod;
pub(super) mod parent_mod;
pub mod public_mod;
"#;

            let expected = r#"pub mod public_mod;

pub use external::bar;

pub(crate) mod internal_mod;

pub(super) mod parent_mod;

pub(super) use parent::foo;

pub(in crate::utils) use internal::baz;

use std::fs;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_multiple_attributes_on_item() {
            let input = r#"use std::fs;
#[cfg(test)]
#[macro_use]
use test_utils;
#[allow(dead_code)]
#[inline]
pub use api::Client;
use std::io;
"#;

            let expected = r#"#[allow(dead_code)]
#[inline]
pub use api::Client;

#[cfg(test)]
#[macro_use]
use test_utils;

use std::fs;
use std::io;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_comment_and_multiple_attributes() {
            let input = r#"use std::fs;
// This is a test utility that needs special handling
#[cfg(test)]
#[macro_use]
use test_utils;
// Public API client
#[allow(dead_code)]
pub use api::Client;
use std::io;
"#;

            let expected = r#"// Public API client
#[allow(dead_code)]
pub use api::Client;

// This is a test utility that needs special handling
#[cfg(test)]
#[macro_use]
use test_utils;

use std::fs;
use std::io;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_empty_file() {
            let input = r#""#;
            let expected = r#""#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_only_comments() {
            let input = r#"// This is just a comment file
// With multiple comment lines
// And no actual code
"#;

            let expected = r#"// This is just a comment file
// With multiple comment lines
// And no actual code
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_only_global_attributes() {
            let input = r#"#![feature(test)]
#![warn(unused_imports)]
#![recursion_limit = "256"]
"#;

            let expected = r#"#![feature(test)]
#![warn(unused_imports)]
#![recursion_limit = "256"]
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_doc_comments() {
            let input = r#"//! This is a module-level doc comment
//! It documents the entire module

use std::fs;
/// This documents the bar import
pub use bar::baz;
/// Documents the HashMap import
use std::collections::HashMap;
"#;

            let expected = r#"//! This is a module-level doc comment
//! It documents the entire module

/// This documents the bar import
pub use bar::baz;

/// Documents the HashMap import
use std::collections::HashMap;

use std::fs;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_comment_blank_line_before_struct() {
            let input = r#"// aaa
// bbb

// ccc

pub struct Alfa;
"#;

            // The blank line between "// ccc" and "pub struct Alfa" should be preserved
            let expected = r#"// aaa
// bbb

// ccc

pub struct Alfa;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }

        #[test]
        fn test_consecutive_cfg_attributes() {
            // Start with properly formatted code - should remain the same
            let input = r#"#[cfg(bla)]
mod test;

#[cfg(bla)]
mod test2;
"#;

            // Should stay the same - exactly ONE blank line between decorated items
            let expected = r#"#[cfg(bla)]
mod test;

#[cfg(bla)]
mod test2;
"#;

            let result = group_items(input).unwrap();
            assert_eq!(result, expected);
        }
    }
}
