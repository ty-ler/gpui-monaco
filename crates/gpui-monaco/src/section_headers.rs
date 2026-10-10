//! Minimap section headers (Monaco `SectionHeaderDetector`): `#region`
//! labels from the language's folding markers and `MARK:` comments, shown
//! as labels in the minimap.
//!
//! Ports `findSectionHeaders.ts` and `sectionHeaders.ts` (MIT).

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, Task};
use syntax::Pattern;
use text::{PointUtf16, Rope};

use crate::editor::CodeEditor;

/// Monaco's `RunOnceScheduler` delay after an edit.
const UPDATE_DELAY: Duration = Duration::from_millis(250);
/// Monaco searches mark headers in chunks of lines that overlap by the
/// longest header.
const CHUNK_SIZE: u32 = 100;
const MAX_SECTION_LINES: u32 = 5;

/// A section header (Monaco `SectionHeader`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionHeader {
    /// Where the header's text is.
    pub range: Range<PointUtf16>,
    /// The label.
    pub text: String,
    /// Draw a separator line above it.
    pub has_separator_line: bool,
    /// Only counts inside comments (mark headers).
    pub should_be_in_comments: bool,
}

/// Monaco `FindSectionHeaderOptions`.
pub struct FindSectionHeaderOptions<'a> {
    /// The language's region start marker.
    pub region_start: Option<&'a Pattern>,
    pub find_region_section_headers: bool,
    pub find_mark_section_headers: bool,
    pub mark_section_header_regex: &'a str,
}

/// Monaco `getHeaderText`: trimmed, with leading and trailing dashes
/// removed; a leading dash means a separator line.
fn header_text(text: &str) -> (String, bool) {
    let text = text.trim();
    let has_separator_line = text.starts_with('-');
    (text.trim_matches('-').to_string(), has_separator_line)
}

/// Monaco `findSectionHeaders`.
pub fn find_section_headers(rope: &Rope, options: &FindSectionHeaderOptions) -> Vec<SectionHeader> {
    let mut headers = Vec::new();
    if options.find_region_section_headers
        && let Some(start) = options.region_start
    {
        headers.extend(collect_region_headers(rope, start));
    }
    if options.find_mark_section_headers {
        headers.extend(collect_mark_headers(
            rope,
            options.mark_section_header_regex,
        ));
    }
    headers
}

fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// Monaco `collectRegionHeaders`: the text after each region start marker.
fn collect_region_headers(rope: &Rope, start: &Pattern) -> Vec<SectionHeader> {
    let Some(regex) = start.regex() else {
        return Vec::new();
    };
    let mut headers = Vec::new();
    for row in 0..rope.line_count() {
        let line = rope.line(row);
        let Ok(Some(found)) = regex.find(&line) else {
            continue;
        };
        // Monaco takes the text after the match's length (its markers
        // start at `^`).
        let len = found.as_str().len();
        let (matched, rest) = line.split_at(len.min(line.len()));
        if rest.is_empty() {
            continue;
        }
        let (text, has_separator_line) = header_text(rest);
        if !text.is_empty() || has_separator_line {
            headers.push(SectionHeader {
                range: PointUtf16::new(row, utf16_len(matched))
                    ..PointUtf16::new(row, utf16_len(&line)),
                text,
                has_separator_line,
                should_be_in_comments: false,
            });
        }
    }
    headers
}

/// Monaco `collectMarkHeaders`: matches of the mark regex (`label` and
/// `separator` groups), searched in overlapping chunks of lines.
fn collect_mark_headers(rope: &Rope, source: &str) -> Vec<SectionHeader> {
    let mut headers: Vec<SectionHeader> = Vec::new();
    if source.trim().is_empty() {
        return headers;
    }
    let multiline = crate::search::is_multiline_regex_source(source);
    let pattern = Pattern::new(source, if multiline { "ms" } else { "m" });
    let Some(regex) = pattern.regex() else {
        return headers;
    };
    let line_count = rope.line_count();
    let mut start_row = 0;
    while start_row < line_count {
        let end_row = (start_row + CHUNK_SIZE - 1).min(line_count - 1);
        let text: String = (start_row..=end_row)
            .map(|row| rope.line(row))
            .collect::<Vec<_>>()
            .join("\n");
        for captures in regex.captures_iter(&text).flatten() {
            let Some(whole) = captures.get(0) else {
                continue;
            };
            // Empty matches can't make headers (Monaco guards against
            // endless loops instead).
            if whole.as_str().is_empty() {
                continue;
            }
            let preceding = &text[..whole.start()];
            let row = start_row + preceding.matches('\n').count() as u32;
            let line_start = preceding.rfind('\n').map_or(0, |i| i + 1);
            let start_column = utf16_len(&text[line_start..whole.start()]);
            let matched_lines: Vec<&str> = whole.as_str().split('\n').collect();
            let end_row = row + matched_lines.len() as u32 - 1;
            let end_column = if matched_lines.len() == 1 {
                start_column + utf16_len(whole.as_str())
            } else {
                utf16_len(matched_lines.last().copied().unwrap_or(""))
            };
            let label = captures.name("label").map_or("", |m| m.as_str());
            let separator = captures
                .name("separator")
                .is_some_and(|m| !m.as_str().is_empty());
            if (label.is_empty() && !separator)
                || headers.last().is_some_and(|h| h.range.end.row >= row)
            {
                continue;
            }
            headers.push(SectionHeader {
                range: PointUtf16::new(row, start_column)..PointUtf16::new(end_row, end_column),
                text: label.to_string(),
                has_separator_line: separator,
                should_be_in_comments: true,
            });
        }
        start_row += CHUNK_SIZE - MAX_SECTION_LINES;
    }
    headers
}

/// Monaco `_fitSectionHeader`: keeps `max_char_count` characters around
/// an ellipsis in the middle.
pub(crate) fn fit_section_header(text: &str, max_char_count: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_char_count {
        return text.to_string();
    }
    let keep = max_char_count;
    let mut half = keep.div_ceil(2);
    while half > 0 && chars[half - 1].is_whitespace() {
        half -= 1;
    }
    let tail = keep - half;
    let mut out: String = chars[..half].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

#[derive(Default)]
pub(crate) struct SectionHeaderState {
    pub headers: Vec<SectionHeader>,
    /// What they were found for: the text, the parsed text (Monaco
    /// recomputes when tokens change, for the comment check), and a hash
    /// of the options and language.
    key: Option<(text::Version, Option<text::Version>, u64)>,
    task: Option<Task<()>>,
}

impl CodeEditor {
    /// Finds the section headers again after edits (Monaco's 250ms
    /// scheduler), in the background. Called before painting.
    pub(crate) fn refresh_section_headers(&mut self, cx: &mut Context<Self>) {
        let minimap = &self.options.minimap;
        let wanted = minimap.enabled
            && !self.options.single_line
            && (minimap.show_region_section_headers || minimap.show_mark_section_headers);
        if !wanted {
            if !self.section_headers.headers.is_empty() {
                self.section_headers = SectionHeaderState::default();
            }
            return;
        }
        let model = self.model().read(cx);
        let options_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            minimap.show_region_section_headers.hash(&mut hasher);
            minimap.show_mark_section_headers.hash(&mut hasher);
            minimap.mark_section_header_regex.hash(&mut hasher);
            model.language_id().hash(&mut hasher);
            hasher.finish()
        };
        let parsed = model.syntax_snapshot().map(|syntax| syntax.text.version());
        let key = (model.buffer().version(), parsed, options_hash);
        if self.section_headers.key.as_ref() == Some(&key) {
            return;
        }
        let first = self.section_headers.key.is_none();
        self.section_headers.key = Some(key);
        let rope = model.buffer().rope().clone();
        let region_start = model
            .language()
            .and_then(|l| l.config.folding.as_ref())
            .and_then(|f| f.markers.as_ref())
            .map(|m| m.start.clone());
        let regex = minimap.mark_section_header_regex.to_string();
        let (find_regions, find_marks) = (
            minimap.show_region_section_headers,
            minimap.show_mark_section_headers,
        );
        self.section_headers.task = Some(cx.spawn(async move |this, cx| {
            if !first {
                cx.background_executor().timer(UPDATE_DELAY).await;
            }
            let headers = cx
                .background_executor()
                .spawn(async move {
                    find_section_headers(
                        &rope,
                        &FindSectionHeaderOptions {
                            region_start: region_start.as_ref(),
                            find_region_section_headers: find_regions,
                            find_mark_section_headers: find_marks,
                            mark_section_header_regex: &regex,
                        },
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                let model = this.model().read(cx);
                if model.buffer().version() != key.0 {
                    return;
                }
                // Monaco keeps mark headers only inside comments.
                let rope = model.buffer().rope();
                let headers = headers
                    .into_iter()
                    .filter(|header| {
                        !header.should_be_in_comments
                            || model.token_type_at(rope.point_utf16_to_offset(header.range.start))
                                == crate::cursor::StandardTokenType::Comment
                    })
                    .collect();
                this.section_headers.headers = headers;
                this.section_headers.task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The minimap's section headers: (row, label, separator), for tests.
    #[doc(hidden)]
    pub fn section_headers_for_tests(&self) -> Vec<(u32, String, bool)> {
        self.section_headers
            .headers
            .iter()
            .map(|h| (h.range.start.row, h.text.clone(), h.has_separator_line))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARK: &str = r"\bMARK:\s*(?<separator>-?)\s*(?<label>.*)$";

    fn find(text: &str, region: Option<&str>) -> Vec<(u32, String, bool)> {
        let region = region.map(|r| Pattern::new(r, ""));
        find_section_headers(
            &Rope::from(text),
            &FindSectionHeaderOptions {
                region_start: region.as_ref(),
                find_region_section_headers: true,
                find_mark_section_headers: true,
                mark_section_header_regex: MARK,
            },
        )
        .into_iter()
        .map(|h| (h.range.start.row, h.text, h.has_separator_line))
        .collect()
    }

    #[test]
    fn region_and_mark_headers() {
        let text = "// #region Setup\nlet a = 1;\n// #endregion\n// MARK: - Helpers\n// MARK: Utilities\n// MARK:\n";
        assert_eq!(
            find(text, Some(r"^\s*//\s*#?region\b")),
            vec![
                (0, "Setup".to_string(), false),
                (3, "Helpers".to_string(), true),
                (4, "Utilities".to_string(), false),
            ]
        );
        // A region with only dashes still separates.
        assert_eq!(
            find("// #region ---\n", Some(r"^\s*//\s*#?region\b")),
            vec![(0, String::new(), true)]
        );
    }

    #[test]
    fn mark_headers_across_chunks() {
        let mut text = String::new();
        for i in 0..250 {
            if i % 50 == 0 {
                text.push_str(&format!("// MARK: Part {i}\n"));
            } else {
                text.push_str("x\n");
            }
        }
        let rows: Vec<u32> = find(&text, None).into_iter().map(|h| h.0).collect();
        // Overlapping chunks don't repeat them.
        assert_eq!(rows, vec![0, 50, 100, 150, 200]);
    }

    #[test]
    fn fitting_shortens_in_the_middle() {
        assert_eq!(fit_section_header("Short", 10), "Short");
        assert_eq!(fit_section_header("Hello wonderful world", 9), "Hello…orld");
    }
}
