//! Backend-agnostic change capture.
//!
//! Two code paths, both producing a [`Diff`] = list of [`FileChange`]:
//!
//! - **Git mode.** When `merged/.git` exists we shell `git diff --no-color
//!   HEAD` plus `git ls-files --others --exclude-standard` (for untracked),
//!   split the output on `diff --git` headers, and emit one [`FileChange`] per
//!   file. Binary entries surface as `diff: None`.
//! - **Plain mode.** No `.git`; we walk both trees in parallel, short-circuit
//!   on `(size, mtime-truncated-to-seconds)` equality, and emit a unified diff
//!   for each surviving pair via `similar`. NUL within the first 8 KiB
//!   classifies the file as binary → `diff: None`.
//!
//! Per the PAL contract: for binary files we don't materialize the bytes
//! in the patch: callers that want them read directly from `merged`
//! (for `Added`/`Modified`) or `lower` (for `Removed`).

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fs::Metadata,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{IsoError, IsoResult};

/// Captured changes between a `lower` baseline and a `merged` view.
#[derive(Debug, Clone, Default)]
pub struct Diff {
    pub files: Vec<FileChange>,
}

impl Diff {
    pub const fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// One entry in a [`Diff`].
///
/// `path` is relative to `merged`. `diff = None` means the file is binary
/// or otherwise text-unrepresentable: copy the contents from the merged
/// tree if you need them (or skip if you only care about text).
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub op: ChangeKind,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
}

/// Default backend diff: git when available, mtime-skipped walk otherwise.
pub async fn default_diff(lower: &Path, merged: &Path) -> IsoResult<Diff> {
    if is_git_tree(merged).await {
        git_diff(merged).await
    } else {
        walk_diff(lower, merged).await
    }
}

async fn is_git_tree(merged: &Path) -> bool {
    tokio::fs::symlink_metadata(merged.join(".git"))
        .await
        .is_ok()
}

// ─── git mode ───────────────────────────────────────────────────────────────

async fn git_diff(merged: &Path) -> IsoResult<Diff> {
    // `--no-color`: keep ANSI out of patch text.
    // No `--binary`: we *want* git's `Binary files … differ` placeholder
    // so we can map it to `diff: None`.
    // The parser reads git's own patch format with `a/` and `b/` header
    // prefixes. Both are pinned on the command: `diff.noprefix` in the user's
    // or repository's git config removes the prefixes, and `diff.external` (or
    // GIT_EXTERNAL_DIFF) replaces the whole patch stream with a viewer's
    // output. Either way a modified file parsed as no change.
    let tracked = git_run(
        merged,
        &[
            "-c",
            "core.quotepath=off",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "HEAD",
        ],
    )
    .await?;

    let untracked_list = git_run(
        merged,
        &[
            "-c",
            "core.quotepath=off",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )
    .await?;

    let mut files = parse_git_diff(&tracked);

    let mut untracked_paths: Vec<&[u8]> = untracked_list
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    untracked_paths.sort_unstable();

    for path_bytes in untracked_paths {
        let path_str = std::str::from_utf8(path_bytes)
            .map_err(|err| IsoError::other(format!("untracked path is not valid UTF-8: {err}")))?;
        let one = git_run_allow_exit1(
            merged,
            &[
                "-c",
                "core.quotepath=off",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--no-index",
                git_null_path(),
                path_str,
            ],
        )
        .await?;
        files.extend(parse_git_diff(&one));
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Diff { files })
}

#[cfg(windows)]
const fn git_null_path() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
const fn git_null_path() -> &'static str {
    "/dev/null"
}

async fn git_run(cwd: &Path, args: &[&str]) -> IsoResult<Vec<u8>> {
    let output = git_spawn(cwd, args).await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(IsoError::other(format!(
            "git {} (exit {}): {stderr}",
            args.join(" "),
            output
                .status
                .code()
                .map_or_else(|| "?".into(), |c| c.to_string())
        )));
    }
    Ok(output.stdout)
}

/// `git diff --no-index` returns exit code 1 when files differ: that's
/// not an error for us, treat it as success with the produced patch.
async fn git_run_allow_exit1(cwd: &Path, args: &[&str]) -> IsoResult<Vec<u8>> {
    let output = git_spawn(cwd, args).await?;
    if output.status.success() || output.status.code() == Some(1) {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(IsoError::other(format!(
        "git {} (exit {}): {stderr}",
        args.join(" "),
        output
            .status
            .code()
            .map_or_else(|| "?".into(), |c| c.to_string())
    )))
}

async fn git_spawn(cwd: &Path, args: &[&str]) -> IsoResult<std::process::Output> {
    let mut cmd = crate::spawn::command("git").map_err(git_spawn_error)?;
    cmd.arg("-C").arg(cwd).args(args);
    cmd.stdin(std::process::Stdio::null());
    crate::spawn::output_async(cmd)
        .await
        .map_err(git_spawn_error)
}

fn git_spawn_error(err: std::io::Error) -> IsoError {
    if err.kind() == std::io::ErrorKind::NotFound {
        IsoError::unavailable("`git` not on PATH; cannot capture diff for git-tracked tree")
    } else {
        IsoError::other(format!("spawn git: {err}"))
    }
}

/// Split a `git diff` blob into per-file [`FileChange`] entries. Each
/// entry covers exactly one `diff --git a/<path> b/<path>` block. Binary
/// blocks are emitted with `diff: None`; the rest carry their original
/// unified-diff slice unchanged so `git apply` produces byte-identical
/// results downstream.
fn parse_git_diff(blob: &[u8]) -> Vec<FileChange> {
    // Git does not require UTF-8 content and emits a Latin-1 hunk byte for
    // byte, so the blob is split into file blocks BEFORE decoding: one
    // undecodable block keeps its change with `diff: None` (the text
    // unrepresentable contract) and every other block parses as before.
    // Decoding the whole buffer first returned an empty diff for the lot,
    // which read as "nothing changed" (Loop R170).
    let mut out = Vec::<FileChange>::new();
    for block in split_git_diff_blocks(blob) {
        match std::str::from_utf8(block) {
            Ok(text) => out.extend(parse_git_diff_text(text)),
            Err(_) => out.extend(parse_undecodable_block(block)),
        }
    }
    out
}

/// Byte-wise split at every `diff --git ` header (at the start or after a
/// newline), so decoding is decided per file rather than per buffer.
fn split_git_diff_blocks(blob: &[u8]) -> Vec<&[u8]> {
    const HEADER: &[u8] = b"diff --git ";
    let mut starts = Vec::new();
    let mut at = 0;
    while at < blob.len() {
        let line_end = blob[at..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(blob.len(), |i| at + i + 1);
        if blob[at..line_end].starts_with(HEADER) {
            starts.push(at);
        }
        at = line_end;
    }
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| &blob[start..starts.get(i + 1).copied().unwrap_or(blob.len())])
        .collect()
}

/// A block whose patch bytes are not UTF-8: keep the path and kind, drop the
/// text. The path is read from the header line only, lossily, because the
/// header is the one line git writes for us rather than from file content.
fn parse_undecodable_block(block: &[u8]) -> Option<FileChange> {
    let header_end = block
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(block.len());
    let header = String::from_utf8_lossy(&block[..header_end]);
    let rest = header.strip_prefix("diff --git ")?;
    let mut path = header_new_path(rest.trim_end_matches('\n'))?;
    let mut op = ChangeKind::Modified;
    for line in block.split(|&b| b == b'\n') {
        if line.starts_with(b"new file mode ") {
            op = ChangeKind::Added;
        } else if line.starts_with(b"deleted file mode ") {
            op = ChangeKind::Removed;
        } else if let Some(renamed) = line
            .strip_prefix(b"rename to ")
            .and_then(|rest| std::str::from_utf8(rest).ok())
        {
            path = PathBuf::from(unquote_git_path(renamed));
        }
    }
    Some(FileChange {
        path,
        op,
        diff: None,
    })
}

/// The new-side path of a `diff --git a/<old> b/<new>` header. Splitting at the
/// first space put the rest of an old filename containing spaces into the
/// reported path, so `my file.bin` came back as `file.bin b/my file.bin`, a
/// path that does not exist (Loop R189). Git writes both sides unquoted unless
/// a name needs C-quoting (a quote, a backslash, a control character); an
/// unquoted header for a modification is symmetric, `a/P b/P`, so the path is
/// the half that makes it so. A quoted header is read as two C strings. A
/// header that is neither (a rename between unquoted names with spaces) is
/// split at the last ` b/`, and the block's `rename to` line then settles it.
fn header_new_path(header: &str) -> Option<PathBuf> {
    if header.starts_with('"') {
        let (_, after_old) = read_c_quoted(header)?;
        let after_old = after_old.strip_prefix(' ')?;
        let new = if after_old.starts_with('"') {
            read_c_quoted(after_old)?.0
        } else {
            after_old.to_string()
        };
        return Some(PathBuf::from(new.strip_prefix("b/").unwrap_or(&new)));
    }
    let len = header.len();
    if len >= 5 && (len - 5).is_multiple_of(2) {
        let n = (len - 5) / 2;
        // The probe is done on bytes: n is a byte count, and under
        // core.quotepath=off a multi-byte name can put 5+n inside a character,
        // where a str slice panics. " b/" is ASCII, so a byte match at 2+n
        // proves both slice edges below fall on character boundaries.
        let bytes = header.as_bytes();
        if header.starts_with("a/")
            && bytes.get(2 + n..5 + n) == Some(b" b/".as_slice())
            && bytes[2..2 + n] == bytes[5 + n..]
        {
            return Some(PathBuf::from(&header[2..2 + n]));
        }
    }
    // A quoted new side after an unquoted old side (`a/old.txt "b/new\"name.txt"`):
    // git quotes each side on its own, so a rename onto a name that needs
    // quoting has no ` b/` separator and the block was dropped whole rather
    // than misnamed (Loop R200). The quoted string ends the header.
    if let Some(at) = header.rfind(" \"") {
        if let Some((new, rest)) = read_c_quoted(&header[at + 1..]) {
            if rest.is_empty() {
                return Some(PathBuf::from(new.strip_prefix("b/").unwrap_or(&new)));
            }
        }
    }
    let at = header.rfind(" b/")?;
    Some(PathBuf::from(&header[at + 3..]))
}

/// A path as git prints it on a `rename to` line: quoted only when it must be.
fn unquote_git_path(raw: &str) -> String {
    if raw.starts_with('"') {
        read_c_quoted(raw)
            .map(|(s, _)| s)
            .unwrap_or_else(|| raw.to_string())
    } else {
        raw.to_string()
    }
}

/// Decode one whole git-quoted pathname strictly: the token must be a complete
/// C-quoted string with nothing after the closing quote, every escape must be
/// one git writes, and the bytes must be UTF-8. Anything else is `None`.
///
/// The lenient readers in this file serve display, where a best effort beats
/// a blank; a caller that fences on the decoded path needs the opposite, since
/// a token it cannot decode is one git may still apply to a path it cannot see.
pub fn decode_git_quoted_path(token: &str) -> Option<String> {
    let mut bytes = token.strip_prefix('"')?.bytes();
    let mut out = Vec::new();
    loop {
        let byte = bytes.next()?;
        match byte {
            b'"' => break,
            b'\\' => match bytes.next()? {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(7),
                b'b' => out.push(8),
                b'f' => out.push(12),
                b'v' => out.push(11),
                b'\\' => out.push(b'\\'),
                b'"' => out.push(b'"'),
                first @ b'0'..=b'7' => {
                    let mut value = u32::from(first - b'0');
                    for _ in 0..2 {
                        let next = bytes.next()?;
                        if !(b'0'..=b'7').contains(&next) {
                            return None;
                        }
                        value = value * 8 + u32::from(next - b'0');
                    }
                    out.push(u8::try_from(value).ok()?);
                }
                _ => return None,
            },
            other => out.push(other),
        }
    }
    if bytes.next().is_some() {
        return None;
    }
    String::from_utf8(out).ok()
}

/// Read one C-quoted string as git writes it (`\\`, `\"`, `\n`, `\t`, octal
/// bytes) and return it with the unread remainder.
fn read_c_quoted(input: &str) -> Option<(String, &str)> {
    let mut bytes = input.strip_prefix('"')?.bytes();
    let mut out = Vec::new();
    let mut consumed = 1;
    loop {
        let byte = bytes.next()?;
        consumed += 1;
        match byte {
            b'"' => break,
            b'\\' => {
                let escaped = bytes.next()?;
                consumed += 1;
                match escaped {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'a' => out.push(7),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'v' => out.push(11),
                    b'0'..=b'7' => {
                        let mut value = u32::from(escaped - b'0');
                        for _ in 0..2 {
                            let Some(next) = input.as_bytes().get(consumed) else {
                                break;
                            };
                            if !(b'0'..=b'7').contains(next) {
                                break;
                            }
                            value = value * 8 + u32::from(next - b'0');
                            bytes.next();
                            consumed += 1;
                        }
                        out.push(value as u8);
                    }
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    Some((
        String::from_utf8_lossy(&out).into_owned(),
        &input[consumed..],
    ))
}

fn parse_git_diff_text(text: &str) -> Vec<FileChange> {
    let mut out = Vec::<FileChange>::new();
    let iter = text.split_inclusive('\n');
    let mut buf = String::new();
    let mut header_path: Option<PathBuf> = None;
    let mut header_kind = ChangeKind::Modified;
    let mut header_binary = false;

    let flush = |buf: &mut String,
                 path: &mut Option<PathBuf>,
                 kind: &mut ChangeKind,
                 binary: &mut bool,
                 out: &mut Vec<FileChange>| {
        if let Some(p) = path.take() {
            let diff = if *binary {
                None
            } else {
                Some(std::mem::take(buf))
            };
            out.push(FileChange {
                path: p,
                op: *kind,
                diff,
            });
        }
        buf.clear();
        *kind = ChangeKind::Modified;
        *binary = false;
    };

    for line in iter {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush(
                &mut buf,
                &mut header_path,
                &mut header_kind,
                &mut header_binary,
                &mut out,
            );
            header_path = header_new_path(rest.trim_end_matches('\n'));
            buf.push_str(line);
            continue;
        }
        if header_path.is_some() {
            if line.starts_with("new file mode ") {
                header_kind = ChangeKind::Added;
            } else if line.starts_with("deleted file mode ") {
                header_kind = ChangeKind::Removed;
            } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
                header_binary = true;
            } else if let Some(renamed) = line.strip_prefix("rename to ") {
                // The rename target is the one line that names the new path
                // alone; it settles a header whose two sides differ.
                header_path = Some(PathBuf::from(unquote_git_path(
                    renamed.trim_end_matches('\n'),
                )));
            }
            buf.push_str(line);
        }
    }
    flush(
        &mut buf,
        &mut header_path,
        &mut header_kind,
        &mut header_binary,
        &mut out,
    );
    out
}

// ─── plain mode ─────────────────────────────────────────────────────────────

async fn walk_diff(lower: &Path, merged: &Path) -> IsoResult<Diff> {
    let lower = lower.to_path_buf();
    let merged = merged.to_path_buf();
    tokio::task::spawn_blocking(move || walk_diff_blocking(&lower, &merged))
        .await
        .map_err(|err| IsoError::other(format!("walk_diff join: {err}")))?
}

fn walk_diff_blocking(lower: &Path, merged: &Path) -> IsoResult<Diff> {
    let lower_index = index_tree(lower)?;
    let merged_index = index_tree(merged)?;

    let mut files: Vec<FileChange> = Vec::new();

    for (rel, m_meta) in &merged_index {
        match lower_index.get(rel) {
            None => files.push(plain_change(merged, rel, ChangeKind::Added, None)?),
            Some(l_meta) => {
                if metas_equal(l_meta, m_meta) {
                    continue;
                }
                files.push(plain_change(
                    merged,
                    rel,
                    ChangeKind::Modified,
                    Some(lower),
                )?);
            }
        }
    }
    for rel in lower_index.keys() {
        if !merged_index.contains_key(rel) {
            files.push(plain_change(lower, rel, ChangeKind::Removed, None)?);
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Diff { files })
}

fn metas_equal(a: &Metadata, b: &Metadata) -> bool {
    if a.len() != b.len() {
        return false;
    }
    match (a.modified(), b.modified()) {
        (Ok(ma), Ok(mb)) => systime_eq(ma, mb),
        _ => false,
    }
}

fn systime_eq(a: SystemTime, b: SystemTime) -> bool {
    // Filesystems carry mtime at different resolutions (HFS+ seconds, APFS
    // nanos, FAT 2 seconds). Compare at second granularity so a metadata-
    // preserving copy that flushed through a coarse layer doesn't look
    // modified.
    let to_secs = |t: SystemTime| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    };
    to_secs(a) == to_secs(b)
}

fn index_tree(root: &Path) -> IsoResult<BTreeMap<PathBuf, Metadata>> {
    let mut out = BTreeMap::new();
    if !root.exists() {
        return Ok(out);
    }
    walk(root, root, &mut out)?;
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Metadata>) -> IsoResult<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| IsoError::other(format!("read_dir {}: {err}", dir.display())))?;
    for entry in entries {
        let entry = entry
            .map_err(|err| IsoError::other(format!("dir entry in {}: {err}", dir.display())))?;
        let path = entry.path();
        let meta = entry
            .metadata()
            .map_err(|err| IsoError::other(format!("metadata {}: {err}", path.display())))?;
        if meta.is_symlink() {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.insert(rel, meta);
            continue;
        }
        if meta.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        out.insert(rel, meta);
    }
    Ok(())
}

/// Build a [`FileChange`] for an entry observed by [`walk_diff_blocking`].
///
/// `op == Modified` requires `peer_root = Some(lower)` so we can read the
/// counterpart; `Added`/`Removed` only need the side we already know about.
fn plain_change(
    side: &Path,
    rel: &Path,
    op: ChangeKind,
    peer_root: Option<&Path>,
) -> IsoResult<FileChange> {
    let full = side.join(rel);
    let primary = std::fs::read(&full)
        .map_err(|err| IsoError::other(format!("read {}: {err}", full.display())))?;
    if looks_binary(&primary) {
        return Ok(FileChange {
            path: rel.to_path_buf(),
            op,
            diff: None,
        });
    }
    let (old_bytes, new_bytes) = match op {
        ChangeKind::Added => (Vec::new(), primary),
        ChangeKind::Removed => (primary, Vec::new()),
        ChangeKind::Modified => {
            let peer = peer_root.expect("modified change requires peer root");
            let peer_full = peer.join(rel);
            let peer_bytes = std::fs::read(&peer_full)
                .map_err(|err| IsoError::other(format!("read {}: {err}", peer_full.display())))?;
            if looks_binary(&peer_bytes) {
                return Ok(FileChange {
                    path: rel.to_path_buf(),
                    op,
                    diff: None,
                });
            }
            (peer_bytes, primary)
        }
    };
    let (Ok(old_text), Ok(new_text)) = (
        std::str::from_utf8(&old_bytes),
        std::str::from_utf8(&new_bytes),
    ) else {
        return Ok(FileChange {
            path: rel.to_path_buf(),
            op,
            diff: None,
        });
    };
    Ok(FileChange {
        path: rel.to_path_buf(),
        op,
        diff: Some(render_unified(rel, op, old_text, new_text)),
    })
}

fn render_unified(rel: &Path, op: ChangeKind, old: &str, new: &str) -> String {
    let rel_str = rel.to_string_lossy();
    let (from_label, to_label) = match op {
        ChangeKind::Added => (String::from("/dev/null"), format!("b/{rel_str}")),
        ChangeKind::Removed => (format!("a/{rel_str}"), String::from("/dev/null")),
        ChangeKind::Modified => (format!("a/{rel_str}"), format!("b/{rel_str}")),
    };
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "diff --git a/{rel_str} b/{rel_str}");
    match op {
        ChangeKind::Added => {
            let _ = writeln!(out, "new file mode 100644");
        }
        ChangeKind::Removed => {
            let _ = writeln!(out, "deleted file mode 100644");
        }
        ChangeKind::Modified => {}
    }
    let body = similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&from_label, &to_label)
        .to_string();
    out.push_str(&body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(path: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "diff --git a/{path} b/{path}\nindex 1111111..2222222 100644\n--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n"
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    // A Latin-1 hunk used to empty the whole diff: decoding happened on the
    // buffer, not per file, so one undecodable block read as no change at all.
    #[test]
    fn a_non_utf8_block_keeps_its_change_without_text_and_leaves_the_others_intact() {
        let mut blob = block("legacy.txt", b"-caf\xe9 before\n+caf\xe9 after\n");
        blob.extend_from_slice(&block("normal.txt", b"-before\n+after\n"));

        let files = parse_git_diff(&blob);

        let paths: Vec<_> = files
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(paths, vec!["legacy.txt", "normal.txt"]);
        assert_eq!(files[0].op, ChangeKind::Modified);
        assert!(
            files[0].diff.is_none(),
            "undecodable patch text is not represented"
        );
        let normal = files[1]
            .diff
            .as_deref()
            .expect("utf-8 block keeps its patch");
        assert!(normal.starts_with("diff --git a/normal.txt b/normal.txt\n"));
        assert!(normal.ends_with("-before\n+after\n"));
    }

    #[test]
    fn an_undecodable_added_file_keeps_its_kind() {
        let mut blob = b"diff --git a/new.txt b/new.txt\nnew file mode 100644\n".to_vec();
        blob.extend_from_slice(b"--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+\xff\n");
        let files = parse_git_diff(&blob);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].op, ChangeKind::Added);
        assert!(files[0].diff.is_none());
    }

    // The header is `a/<old> b/<new>` with spaces unquoted, so the new path is
    // the half that makes the header symmetric, a rename is settled by its
    // `rename to` line, and a quoted header is read as C strings (Loop R189:
    // splitting at the first space returned `file.bin b/my file.bin`).
    #[test]
    fn header_paths_survive_spaces_quotes_and_renames() {
        let binary = b"diff --git a/my file.bin b/my file.bin\nindex 1..2 100644\nBinary files a/my file.bin and b/my file.bin differ\n";
        let files = parse_git_diff(binary);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("my file.bin"));
        assert!(files[0].diff.is_none());

        let renamed = b"diff --git a/a b.txt b/c d.txt\nsimilarity index 100%\nrename from a b.txt\nrename to c d.txt\n";
        let files = parse_git_diff(renamed);
        assert_eq!(files[0].path, PathBuf::from("c d.txt"));

        let quoted = b"diff --git \"a/we\\\"ird.txt\" \"b/we\\\"ird.txt\"\nindex 1..2 100644\n--- \"a/we\\\"ird.txt\"\n+++ \"b/we\\\"ird.txt\"\n@@ -1 +1 @@\n-x\n+y\n";
        let files = parse_git_diff(quoted);
        assert_eq!(files[0].path, PathBuf::from("we\"ird.txt"));

        // A rename onto a name git quotes: the old side is bare, the new side
        // is a C string, and the block was dropped whole (Loop R200).
        let onto_quoted = b"diff --git a/old.txt \"b/new\\\"name.txt\"\nsimilarity index 100%\nrename from old.txt\nrename to \"new\\\"name.txt\"\n";
        let files = parse_git_diff(onto_quoted);
        assert_eq!(files.len(), 1, "the rename is a change");
        assert_eq!(files[0].path, PathBuf::from("new\"name.txt"));
        let onto_tab = b"diff --git a/old.txt \"b/new\\tname.txt\"\nsimilarity index 100%\nrename from old.txt\nrename to \"new\\tname.txt\"\n";
        let files = parse_git_diff(onto_tab);
        assert_eq!(files[0].path, PathBuf::from("new\tname.txt"));
        // And the reverse: a quoted old side onto a bare new side.
        let from_quoted = b"diff --git \"a/we\\\"ird.txt\" b/plain.txt\nsimilarity index 100%\nrename from \"we\\\"ird.txt\"\nrename to plain.txt\n";
        let files = parse_git_diff(from_quoted);
        assert_eq!(files[0].path, PathBuf::from("plain.txt"));

        // A rename onto a multi-byte name, unquoted under core.quotepath=off:
        // the symmetric-header probe once sliced the string at a byte count
        // that fell inside the character and panicked instead of reaching the
        // rename line.
        let unicode = "diff --git a/old b/\u{65b0}.txt\nsimilarity index 100%\nrename from old\nrename to \u{65b0}.txt\n";
        let files = parse_git_diff(unicode.as_bytes());
        assert_eq!(files.len(), 1, "the rename is a change");
        assert_eq!(files[0].path, PathBuf::from("\u{65b0}.txt"));
        let ascii_control = b"diff --git a/old b/new.txt\nsimilarity index 100%\nrename from old\nrename to new.txt\n";
        assert_eq!(
            parse_git_diff(ascii_control)[0].path,
            PathBuf::from("new.txt")
        );

        // The undecodable arm shares the header reader.
        let mut undecodable =
            b"diff --git a/my file.bin b/my file.bin\nindex 1..2 100644\n".to_vec();
        undecodable.extend_from_slice(
            b"--- a/my file.bin\n+++ b/my file.bin\n@@ -1 +1 @@\n-\xff\n+\xfe\n",
        );
        let files = parse_git_diff(&undecodable);
        assert_eq!(files[0].path, PathBuf::from("my file.bin"));
    }
}
