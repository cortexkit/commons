//! Names of the form `ck-<name>` are reserved for production installs. Tests run
//! `ckdev-<name>` copies made by `ckdev_binary` instead, so a test process never
//! looks like a production module in process listings.
//!
//! This guard scans the caller's Rust test directories as text and reports
//! every place a test runs a `ck-*` binary directly instead of through
//! `ckdev_binary`:
//! - text inside comments never counts, so a doc example naming a `ck-*` path
//!   is not a violation; string literals are also ignored when looking for
//!   calls such as `Command::new`;
//! - each `;`-terminated statement is judged on its own, so a correct spawn
//!   does not excuse a direct one in the next statement;
//! - a statement is flagged when it runs, symlinks or wraps a
//!   `CARGO_BIN_EXE_ck-*` path or a `target/.../ck-` path that is not inside a
//!   `ckdev_binary(...)` call;
//! - a `let` binding holding such a path is remembered, so a later
//!   `Command::new(that_binding)` is flagged too.
//!
//! It recognises only the spawn shapes these tests use. The planted controls
//! at the bottom prove it fires on a direct spawn and on a direct spawn right
//! after a wrapped one, and stays quiet on a wrapped spawn and on paths that
//! appear only in comments.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// Blanks comments and string or char literals, leaving only code.
fn mask_non_code(source: &str) -> String {
    mask(source, true)
}

/// Blanks comments only. String literals stay, because that's where a real
/// path such as `"target/debug/ck-subc"` lives.
fn mask_comments(source: &str) -> String {
    mask(source, false)
}

/// Literals are always skipped over, so a `//` inside a string never starts a
/// comment; they are blanked only when `mask_literals` is set.
fn mask(source: &str, mask_literals: bool) -> String {
    let bytes = source.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            let start = index;
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            mask_range(&mut masked, start, index);
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            let start = index;
            index += 2;
            let mut depth = 1usize;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            mask_range(&mut masked, start, index);
            continue;
        }

        if let Some((quote, hashes)) = raw_string_start(bytes, index) {
            let start = index;
            index = quote + 1;
            while index < bytes.len() {
                if bytes[index] == b'"' && bytes[index + 1..].starts_with(&vec![b'#'; hashes]) {
                    index += 1 + hashes;
                    break;
                }
                index += 1;
            }
            if mask_literals {
                mask_range(&mut masked, start, index);
            }
            continue;
        }

        if bytes[index] == b'"' || (bytes[index] == b'b' && bytes.get(index + 1) == Some(&b'"')) {
            let start = index;
            if bytes[index] == b'b' {
                index += 1;
            }
            index += 1;
            while index < bytes.len() {
                match bytes[index] {
                    b'\\' => index = (index + 2).min(bytes.len()),
                    b'"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
            if mask_literals {
                mask_range(&mut masked, start, index);
            }
            continue;
        }

        // Character literals are short; a lifetime such as 'static has no
        // closing quote in this bounded scan and remains ordinary code.
        if bytes[index] == b'\'' {
            if let Some(end) = char_literal_end(bytes, index) {
                if mask_literals {
                    mask_range(&mut masked, index, end);
                }
                index = end;
                continue;
            }
        }
        index += 1;
    }

    String::from_utf8(masked).expect("masking preserves UTF-8 boundaries")
}

fn mask_range(masked: &mut [u8], start: usize, end: usize) {
    for byte in &mut masked[start..end] {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}

fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let mut cursor = index;
    if bytes.get(cursor..cursor + 2) == Some(b"br") {
        cursor += 2;
    } else if bytes.get(cursor) == Some(&b'r') {
        cursor += 1;
    } else {
        return None;
    }
    let hashes_start = cursor;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    Some((cursor, cursor - hashes_start))
}

fn char_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let limit = (start + 12).min(bytes.len());
    let mut cursor = start + 1;
    while cursor < limit && bytes[cursor] != b'\n' {
        if bytes[cursor] == b'\\' {
            cursor += 2;
        } else if bytes[cursor] == b'\'' {
            return Some(cursor + 1);
        } else {
            cursor += 1;
        }
    }
    None
}

fn statements(source: &str) -> Vec<(usize, &str)> {
    let code = mask_non_code(source);
    let mut result = Vec::new();
    let mut start = 0;
    let mut line = 1;
    for (index, byte) in code.bytes().enumerate() {
        if byte == b';' {
            if !code[start..index].trim().is_empty() {
                result.push((line, &source[start..=index]));
            }
            line += source[start..=index]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
            start = index + 1;
        }
    }
    if !code[start..].trim().is_empty() {
        result.push((line, &source[start..]));
    }
    result
}

fn env_binary_marker(statement: &str, code: &str) -> Option<usize> {
    let visible = mask_comments(statement);
    let index = visible
        .find("CARGO_BIN_EXE_ck-")
        .or_else(|| visible.find("CARGO_BIN_EXE_ck\""))?;
    let quote = visible[..index].rfind('"')?;
    code[..quote].trim_end().ends_with("env!(").then_some(index)
}

fn helper_ranges(code: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut search_from = 0;
    while let Some(relative) = code[search_from..].find("ckdev_binary") {
        let start = search_from + relative;
        let Some(open_relative) = code[start..].find('(') else {
            break;
        };
        let open = start + open_relative;
        let mut depth = 0usize;
        let mut end = None;
        for (index, byte) in code.bytes().enumerate().skip(open) {
            if byte == b'(' {
                depth += 1;
            } else if byte == b')' {
                depth -= 1;
                if depth == 0 {
                    end = Some(index + 1);
                    break;
                }
            }
        }
        if let Some(end) = end {
            ranges.push(start..end);
            search_from = end;
        } else {
            break;
        }
    }
    ranges
}

fn statement_has_unwrapped_binary(statement: &str) -> bool {
    let code = mask_non_code(statement);
    // Paths are looked for in text with comments blanked; positions line up
    // with `code`, because masking never changes the length.
    let visible = mask_comments(statement);
    let mut markers = Vec::new();
    if let Some(index) = env_binary_marker(statement, &code) {
        if code.contains("Command::new")
            || code.contains("ckdev_binary")
            || code.contains("symlink")
        {
            markers.push(index);
        }
    }

    if visible.contains("target/")
        && visible.contains("ck-")
        && (code.contains("Command::new")
            || code.contains("ckdev_binary")
            || code.contains("symlink"))
    {
        if let Some(index) = visible.find("target/") {
            markers.push(index);
        }
    }

    if code.contains("symlink") && visible.contains("ck-") {
        if let Some(index) = visible.find("ck-") {
            markers.push(index);
        }
    }

    let ranges = helper_ranges(&code);
    markers
        .iter()
        .any(|marker| !ranges.iter().any(|range| range.contains(marker)))
}

fn binding_name(code: &str) -> Option<String> {
    let let_start = code.find("let ")? + "let ".len();
    let binding = code[let_start..].trim_start();
    let binding = binding.strip_prefix("mut ").unwrap_or(binding);
    let name: String = binding
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn helper_binding(code: &str) -> Option<String> {
    let binding = binding_name(code)?;
    let let_start = code.find("let ")?;
    let equals = code[let_start..].find('=')? + let_start;
    let rhs = code[equals + 1..].trim_start();
    if rhs.starts_with("ckdev_binary(") && !helper_ranges(code).is_empty() {
        Some(binding)
    } else {
        None
    }
}

fn command_argument<'a>(statement: &'a str, code: &str) -> Option<&'a str> {
    let call = code.find("Command::new")?;
    let open = code[call..].find('(')? + call;
    let mut depth = 0usize;
    for (index, byte) in code.bytes().enumerate().skip(open) {
        if byte == b'(' {
            depth += 1;
        } else if byte == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(statement[open + 1..index].trim());
            }
        } else if byte == b',' && depth == 1 {
            return Some(statement[open + 1..index].trim());
        }
    }
    None
}

fn simple_name(expression: &str) -> Option<&str> {
    let expression = expression.trim().trim_start_matches('&').trim();
    let expression = expression
        .strip_prefix('(')
        .and_then(|expression| expression.strip_suffix(')'))
        .unwrap_or(expression)
        .trim();
    (!expression.is_empty()
        && expression
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_'))
    .then_some(expression)
}

fn violations(path: &str, source: &str) -> Vec<String> {
    let mut failures = Vec::new();
    let mut unwrapped_paths = HashSet::new();
    let mut wrapped_paths = HashSet::new();

    for (line, statement) in statements(source) {
        let code = mask_non_code(statement);
        let helper_variable = helper_binding(&code);
        if let Some(variable) = helper_variable {
            wrapped_paths.insert(variable);
        }

        let visible = mask_comments(statement);
        let raw_path = env_binary_marker(statement, &code).is_some()
            || (visible.contains("target/")
                && visible.contains("ck-")
                && binding_name(&code).is_some());
        if raw_path && !code.contains("ckdev_binary") {
            if let Some(variable) = binding_name(&code) {
                unwrapped_paths.insert(variable);
            }
        }

        let indirect_spawn = command_argument(statement, &code)
            .and_then(simple_name)
            .is_some_and(|variable| {
                unwrapped_paths.contains(variable) && !wrapped_paths.contains(variable)
            });
        if statement_has_unwrapped_binary(statement) || indirect_spawn {
            failures.push(format!(
                "{path}:{line}: binary source is not passed through ckdev_binary"
            ));
        }
    }
    failures
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("read test directory entry").path();
        let metadata = std::fs::symlink_metadata(&path).expect("inspect test source path");
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// Assert that Rust test sources in these directories stage production-named
/// artifacts with `ckdev_binary` before spawning them. Comments are ignored.
/// This lexical guard covers direct paths and simple raw-path bindings, not
/// arbitrary data flow. Pair it with `dev_command` for runtime name refusal.
/// Missing directories or an empty source scan panic rather than pass silently.
///
/// ```no_run
/// cortexkit_test_support::assert_test_binary_spawns(&[
///     std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"),
/// ]);
/// ```
pub fn assert_test_binary_spawns(test_directories: &[impl AsRef<Path>]) {
    let mut files = Vec::new();
    for root in test_directories {
        rust_files(root.as_ref(), &mut files);
    }
    assert!(!files.is_empty(), "spawn guard found no Rust test sources");

    let mut failures = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        failures.extend(violations(&path.display().to_string(), &source));
    }
    assert!(
        failures.is_empty(),
        "unwrapped test binaries:\n{}",
        failures.join("\n")
    );
}

#[cfg(test)]
#[test]
fn binary_spawn_guard_controls_reject_direct_and_adjacent_spawns() {
    let direct = r#"
        fn test() {
            Command::new(env!("CARGO_BIN_EXE_ck-example")).spawn();
        }
    "#;
    assert!(!violations("direct.rs", direct).is_empty());

    let direct_cli = r#"fn test() { Command::new(env!("CARGO_BIN_EXE_ck")).spawn(); }"#;
    assert!(!violations("cli.rs", direct_cli).is_empty());

    let adjacent = r#"
        fn test() {
            let safe = ckdev_binary(env!("CARGO_BIN_EXE_ck-example"));
            Command::new(env!("CARGO_BIN_EXE_ck-example")).spawn();
        }
    "#;
    assert!(
        !violations("adjacent.rs", adjacent).is_empty(),
        "a wrapped statement must not bless the next statement"
    );

    let wrapped = r#"
        fn test() {
            Command::new(ckdev_binary(env!("CARGO_BIN_EXE_ck-example"))).spawn();
        }
    "#;
    assert!(violations("wrapped.rs", wrapped).is_empty());

    let commented = r#"
        /// Copies `ck-<name>` into `target/` as `ckdev-<name>`.
        pub fn ckdev_binary(source: &Path) -> PathBuf {
            // env!("CARGO_BIN_EXE_ck-example") is never spawned here.
            let copy = Command::new("cp").arg(source).status();
        }
    "#;
    assert!(
        violations("commented.rs", commented).is_empty(),
        "a path named only in a comment is not a spawn"
    );

    let literal = r#"
        fn test() {
            Command::new("target/debug/ck-subc").spawn();
        }
    "#;
    assert!(
        !violations("literal.rs", literal).is_empty(),
        "a path in a string literal is still judged"
    );
}

#[cfg(test)]
#[test]
fn consumer_scan_rejects_raw_bindings_and_ignores_doc_comments() {
    let scratch = crate::ScratchDir::new("spawn-guard");
    let file = scratch.join("fixture.rs");
    std::fs::write(&file, "/// Command::new(env!(\"CARGO_BIN_EXE_ck-example\")).spawn();\n/* target/debug/ck-example */\nfn test() { Command::new(ckdev_binary(env!(\"CARGO_BIN_EXE_ck-example\"))).spawn(); }\n").unwrap();
    assert_test_binary_spawns(&[scratch.path()]);
    std::fs::write(
        &file,
        "fn test() { let raw = env!(\"CARGO_BIN_EXE_ck-example\"); Command::new(raw).spawn(); }\n",
    )
    .unwrap();
    let failure = std::panic::catch_unwind(|| assert_test_binary_spawns(&[scratch.path()]));
    assert!(failure.is_err(), "raw bound path must be refused");
}
