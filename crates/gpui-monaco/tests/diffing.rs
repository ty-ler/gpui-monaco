//! Port of Monaco's diffing fixture test (`fixtures.test.ts`) for the
//! default ("advanced") and the legacy diff computer: each directory in
//! `tests/fixtures/diffing` holds two inputs (`1.*`, `2.*`) and the results
//! Monaco computes for them (`advanced.expected.diff.json`,
//! `legacy.expected.diff.json`).

use std::fs;
use std::path::{Path, PathBuf};

use gpui_monaco::diff::{
    DetailedLineRangeMapping, DiffOptions, LinesDiff, Range, compute_diff, compute_legacy_diff,
};
use serde_json::{Value, json};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diffing")
}

fn find_file(dir: &Path, prefix: &str) -> (String, String) {
    let name = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .find(|name| name.starts_with(prefix))
        .unwrap_or_else(|| panic!("no {prefix}* file in {}", dir.display()));
    let content = fs::read_to_string(dir.join(&name))
        .unwrap()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    (name, content)
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn format_range(range: &Range, lines: &[&str]) -> String {
    let to_last_char = range.end_column == utf16_len(lines[range.end_line_number - 1]) + 1;
    format!(
        "[{},{} -> {},{}{}]",
        range.start_line_number,
        range.start_column,
        range.end_line_number,
        range.end_column,
        if to_last_char { " EOL" } else { "" }
    )
}

/// The two diff computers Monaco's fixtures cover.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Algorithm {
    Advanced,
    Legacy,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Advanced => "advanced",
            Self::Legacy => "legacy",
        }
    }
}

fn diffs_json(
    changes: &[DetailedLineRangeMapping],
    original: &[&str],
    modified: &[&str],
    algorithm: Algorithm,
) -> Value {
    Value::Array(
        changes
            .iter()
            .map(|c| {
                for pair in c.inner_changes.windows(2) {
                    assert!(
                        pair[0].original_range.end() <= pair[1].original_range.start()
                            && pair[0].modified_range.end() <= pair[1].modified_range.start(),
                        "range mappings must be sorted"
                    );
                }
                // The legacy computer's `undefined` inner changes are
                // serialized as `null`.
                let inner_changes = if algorithm == Algorithm::Legacy && c.inner_changes.is_empty()
                {
                    Value::Null
                } else {
                    c.inner_changes
                        .iter()
                        .map(|i| {
                            json!({
                                "originalRange": format_range(&i.original_range, original),
                                "modifiedRange": format_range(&i.modified_range, modified),
                            })
                        })
                        .collect()
                };
                json!({
                    "originalRange": c.original.to_string(),
                    "modifiedRange": c.modified.to_string(),
                    "innerChanges": inner_changes,
                })
            })
            .collect(),
    )
}

/// UTF-16 offset of a 1-based position in `lines` joined with `\n`.
fn offset_of(lines: &[Vec<u16>], line_number: usize, column: usize) -> usize {
    lines[..line_number - 1]
        .iter()
        .map(|l| l.len() + 1)
        .sum::<usize>()
        + column
        - 1
}

/// Monaco's `assertDiffCorrectness`: applying all inner changes to the
/// original must yield the modified text.
fn assert_diff_correctness(diff: &LinesDiff, original: &[&str], modified: &[&str]) {
    let to_utf16 = |lines: &[&str]| -> Vec<Vec<u16>> {
        lines.iter().map(|l| l.encode_utf16().collect()).collect()
    };
    let original16 = to_utf16(original);
    let modified16 = to_utf16(modified);
    let modified_text: Vec<u16> = modified.join("\n").encode_utf16().collect();
    let mut text: Vec<u16> = original.join("\n").encode_utf16().collect();

    let mut edits: Vec<_> = diff.changes.iter().flat_map(|c| &c.inner_changes).collect();
    edits.sort_by_key(|e| e.original_range.start());
    for edit in edits.iter().rev() {
        let o = &edit.original_range;
        let m = &edit.modified_range;
        let start = offset_of(&original16, o.start_line_number, o.start_column);
        let end = offset_of(&original16, o.end_line_number, o.end_column);
        let new_start = offset_of(&modified16, m.start_line_number, m.start_column);
        let new_end = offset_of(&modified16, m.end_line_number, m.end_column);
        text.splice(
            start..end,
            modified_text[new_start..new_end].iter().copied(),
        );
    }
    assert!(
        text == modified_text,
        "applying the inner changes does not yield the modified text"
    );
}

/// Runs one fixture; returns an error message on mismatch.
fn run_fixture(dir: &Path, algorithm: Algorithm) -> Result<(), String> {
    let folder = dir.file_name().unwrap().to_str().unwrap();
    let (first_name, first_content) = find_file(dir, "1.");
    let (second_name, second_content) = find_file(dir, "2.");
    let first_lines: Vec<&str> = first_content.split('\n').collect();
    let second_lines: Vec<&str> = second_content.split('\n').collect();

    let ignore_trim_whitespace = folder.contains("trimws");
    let options = DiffOptions {
        ignore_trim_whitespace,
        // `Number.MAX_SAFE_INTEGER` in Monaco: effectively no limit.
        max_computation_time_ms: 9_007_199_254_740_991,
        compute_moves: true,
        extend_to_subwords: false,
    };
    let diff = match algorithm {
        Algorithm::Advanced => compute_diff(&first_lines, &second_lines, &options),
        Algorithm::Legacy => compute_legacy_diff(&first_lines, &second_lines, &options),
    };

    // Like Monaco, only the advanced computer's inner changes are checked.
    if algorithm == Algorithm::Advanced && !ignore_trim_whitespace {
        assert_diff_correctness(&diff, &first_lines, &second_lines);
    }

    let mut actual = json!({
        "original": { "content": first_content, "fileName": format!("./{first_name}") },
        "modified": { "content": second_content, "fileName": format!("./{second_name}") },
        "diffs": diffs_json(&diff.changes, &first_lines, &second_lines, algorithm),
    });
    if !diff.moves.is_empty() {
        actual["moves"] = diff
            .moves
            .iter()
            .map(|m| {
                json!({
                    "originalRange": m.line_range_mapping.original.to_string(),
                    "modifiedRange": m.line_range_mapping.modified.to_string(),
                    "changes": diffs_json(&m.changes, &first_lines, &second_lines, algorithm),
                })
            })
            .collect();
    }

    let expected_file = format!("{}.expected.diff.json", algorithm.name());
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(expected_file)).unwrap()).unwrap();
    if actual == expected {
        Ok(())
    } else {
        let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap();
        Err(format!(
            "--- expected\n{}\n--- actual\n{}",
            pretty(&expected),
            pretty(&actual)
        ))
    }
}

/// Runs all fixtures with `algorithm`.
fn run_fixtures(algorithm: Algorithm) {
    // Set `DIFF_FIXTURE=<name>` to run (and print the mismatch of) one fixture.
    let only = std::env::var("DIFF_FIXTURE").ok();
    let mut dirs: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .filter(|path| only.as_deref().is_none_or(|only| path.ends_with(only)))
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty());

    let mut failures = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if let Err(message) = run_fixture(dir, algorithm) {
            if only.is_some() {
                eprintln!("{message}");
            }
            failures.push(name);
        }
    }
    eprintln!(
        "{} of {} {} diffing fixtures pass",
        dirs.len() - failures.len(),
        dirs.len(),
        algorithm.name()
    );
    assert!(failures.is_empty(), "failing fixtures: {failures:?}");
}

#[test]
fn diffing_fixtures() {
    run_fixtures(Algorithm::Advanced);
}

#[test]
fn legacy_diffing_fixtures() {
    run_fixtures(Algorithm::Legacy);
}
