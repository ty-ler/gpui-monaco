//! Snippet variables (`$TM_FILENAME`, `$CURRENT_YEAR`, …).
//!
//! Ports the resolvers of Monaco's
//! `src/vs/editor/contrib/snippet/browser/snippetVariables.ts` (MIT):
//! selection, model, clipboard, comment, time and random based. Workspace
//! variables (`WORKSPACE_NAME`, `WORKSPACE_FOLDER`) come from
//! [`SnippetVariableContext::workspace`], which hosts set; there is no
//! overtyping capture, and day and month names are English.

use chrono::{DateTime, Datelike, Local, Timelike};

/// Every variable name Monaco knows (`KnownSnippetVariableNames`). Unknown
/// names are inserted as placeholders.
pub const KNOWN_VARIABLE_NAMES: &[&str] = &[
    "CURRENT_YEAR",
    "CURRENT_YEAR_SHORT",
    "CURRENT_MONTH",
    "CURRENT_DATE",
    "CURRENT_HOUR",
    "CURRENT_MINUTE",
    "CURRENT_SECOND",
    "CURRENT_MILLISECOND",
    "CURRENT_DAY_NAME",
    "CURRENT_DAY_NAME_SHORT",
    "CURRENT_MONTH_NAME",
    "CURRENT_MONTH_NAME_SHORT",
    "CURRENT_SECONDS_UNIX",
    "CURRENT_MILLISECONDS_UNIX",
    "CURRENT_TIMEZONE_OFFSET",
    "CURRENT_TIMEZONE_NAME",
    "SELECTION",
    "CLIPBOARD",
    "TM_SELECTED_TEXT",
    "TM_CURRENT_LINE",
    "TM_CURRENT_WORD",
    "TM_LINE_INDEX",
    "TM_LINE_NUMBER",
    "TM_FILENAME",
    "TM_FILENAME_BASE",
    "TM_DIRECTORY",
    "TM_DIRECTORY_BASE",
    "TM_FILEPATH",
    "CURSOR_INDEX",
    "CURSOR_NUMBER",
    "RELATIVE_FILEPATH",
    "BLOCK_COMMENT_START",
    "BLOCK_COMMENT_END",
    "LINE_COMMENT",
    "WORKSPACE_NAME",
    "WORKSPACE_FOLDER",
    "RANDOM",
    "RANDOM_HEX",
    "UUID",
];

/// The workspace a document belongs to, for `WORKSPACE_NAME`,
/// `WORKSPACE_FOLDER` and `RELATIVE_FILEPATH`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnippetWorkspace {
    /// Its name.
    pub name: String,
    /// Its root folder's path.
    pub folder: String,
}

/// What variables resolve against at one cursor.
#[derive(Clone, Debug, Default)]
pub struct SnippetVariableContext {
    /// The selected text (`SELECTION`, `TM_SELECTED_TEXT`).
    pub selected_text: String,
    /// Whether the selection spans lines.
    pub selection_is_multiline: bool,
    /// Leading whitespace of the selection's first line, up to its start.
    pub selection_line_leading_whitespace: String,
    /// The cursor's line (`TM_CURRENT_LINE`).
    pub current_line: String,
    /// The word at the cursor (`TM_CURRENT_WORD`).
    pub current_word: Option<String>,
    /// The cursor's 0-based line (`TM_LINE_INDEX`).
    pub line_index: u32,
    /// This cursor's index among all cursors (`CURSOR_INDEX`).
    pub cursor_index: usize,
    /// How many cursors there are (for the clipboard spread).
    pub cursor_count: usize,
    /// The document's path, if it has a `file://` URI.
    pub path: Option<String>,
    /// The document's URI, if it has one that isn't a file.
    pub uri: Option<String>,
    /// The workspace, if the host set one.
    pub workspace: Option<SnippetWorkspace>,
    /// The clipboard text (`CLIPBOARD`).
    pub clipboard: Option<String>,
    /// Whether each cursor gets one clipboard line when the line count
    /// matches the cursor count (`multiCursorPaste: spread`).
    pub clipboard_spread: bool,
    /// The language's line comment (`LINE_COMMENT`).
    pub line_comment: Option<String>,
    /// The language's block comment (`BLOCK_COMMENT_START`, `_END`).
    pub block_comment: Option<(String, String)>,
    /// The time the time variables use (fixed for one insertion).
    pub now: Option<DateTime<Local>>,
}

const DAY_NAMES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Splits text into lines at `\r\n`, `\r` or `\n`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                lines.push(&text[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = i;
            }
            b'\n' => {
                lines.push(&text[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    lines.push(&text[start..]);
    lines
}

/// `path.basename`.
fn basename(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    path.rsplit('/').next().unwrap_or(path)
}

/// `path.dirname`: "." when there is no directory.
fn dirname(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(0) => "/",
        Some(ix) => &path[..ix],
        None => ".",
    }
}

impl SnippetVariableContext {
    /// The value of variable `name`, or `None` if it's unknown or has no
    /// value here (Monaco's `CompositeSnippetVariableResolver`).
    /// `variable_leading_whitespace` is the indentation of the line the
    /// variable is on within the snippet, used to indent a multi-line
    /// selection.
    pub fn resolve(&self, name: &str, variable_leading_whitespace: Option<&str>) -> Option<String> {
        self.resolve_model(name)
            .or_else(|| self.resolve_clipboard(name))
            .or_else(|| self.resolve_selection(name, variable_leading_whitespace))
            .or_else(|| self.resolve_comment(name))
            .or_else(|| self.resolve_time(name))
            .or_else(|| self.resolve_workspace(name))
            .or_else(|| resolve_random(name))
    }

    /// Monaco `SelectionBasedVariableResolver`.
    fn resolve_selection(
        &self,
        name: &str,
        variable_leading_whitespace: Option<&str>,
    ) -> Option<String> {
        match name {
            "SELECTION" | "TM_SELECTED_TEXT" => {
                if self.selected_text.is_empty() {
                    return None;
                }
                let value = self.selected_text.clone();
                match variable_leading_whitespace {
                    Some(var_leading) if self.selection_is_multiline => {
                        // Lines after the first get the extra indentation the
                        // variable has over the selection's line.
                        let line_leading = &self.selection_line_leading_whitespace;
                        let common = var_leading
                            .chars()
                            .zip(line_leading.chars())
                            .take_while(|(a, b)| a == b)
                            .count();
                        let extra: String = var_leading.chars().skip(common).collect();
                        let mut out = String::new();
                        let mut rest = value.as_str();
                        while let Some(ix) = rest.find(['\r', '\n']) {
                            let newline_len = if rest[ix..].starts_with("\r\n") { 2 } else { 1 };
                            out.push_str(&rest[..ix + newline_len]);
                            out.push_str(&extra);
                            rest = &rest[ix + newline_len..];
                        }
                        out.push_str(rest);
                        Some(out)
                    }
                    _ => Some(value),
                }
            }
            "TM_CURRENT_LINE" => Some(self.current_line.clone()),
            "TM_CURRENT_WORD" => self.current_word.clone().filter(|w| !w.is_empty()),
            "TM_LINE_INDEX" => Some(self.line_index.to_string()),
            "TM_LINE_NUMBER" => Some((self.line_index + 1).to_string()),
            "CURSOR_INDEX" => Some(self.cursor_index.to_string()),
            "CURSOR_NUMBER" => Some((self.cursor_index + 1).to_string()),
            _ => None,
        }
    }

    /// Monaco `ModelBasedVariableResolver`, with paths labelled as plain
    /// paths (or the URI for documents that aren't files).
    fn resolve_model(&self, name: &str) -> Option<String> {
        let path = self.path.as_deref();
        match name {
            "TM_FILENAME" => Some(basename(path.or(self.uri.as_deref())?).to_string()),
            "TM_FILENAME_BASE" => {
                let name = basename(path.or(self.uri.as_deref())?);
                Some(match name.rfind('.') {
                    Some(ix) if ix > 0 => name[..ix].to_string(),
                    _ => name.to_string(),
                })
            }
            "TM_DIRECTORY" => {
                let dir = dirname(path?);
                Some(if dir == "." {
                    String::new()
                } else {
                    dir.to_string()
                })
            }
            "TM_DIRECTORY_BASE" => {
                let dir = dirname(path?);
                Some(if dir == "." {
                    String::new()
                } else {
                    basename(dir).to_string()
                })
            }
            "TM_FILEPATH" => path.or(self.uri.as_deref()).map(str::to_string),
            "RELATIVE_FILEPATH" => {
                let path = path.or(self.uri.as_deref())?;
                let relative = self.workspace.as_ref().and_then(|workspace| {
                    path.strip_prefix(&workspace.folder)
                        .map(|rest| rest.trim_start_matches('/').to_string())
                });
                Some(relative.unwrap_or_else(|| path.to_string()))
            }
            _ => None,
        }
    }

    /// Monaco `ClipboardBasedVariableResolver`.
    fn resolve_clipboard(&self, name: &str) -> Option<String> {
        if name != "CLIPBOARD" {
            return None;
        }
        let text = self.clipboard.as_ref().filter(|t| !t.is_empty())?;
        if self.clipboard_spread {
            let lines: Vec<&str> = split_lines(text)
                .into_iter()
                .filter(|line| !line.trim().is_empty())
                .collect();
            if lines.len() == self.cursor_count {
                return Some(lines[self.cursor_index].to_string());
            }
        }
        Some(text.clone())
    }

    /// Monaco `CommentBasedVariableResolver`.
    fn resolve_comment(&self, name: &str) -> Option<String> {
        let non_empty = |s: &String| (!s.is_empty()).then(|| s.clone());
        match name {
            "LINE_COMMENT" => self.line_comment.as_ref().and_then(non_empty),
            "BLOCK_COMMENT_START" => self.block_comment.as_ref().and_then(|(s, _)| non_empty(s)),
            "BLOCK_COMMENT_END" => self.block_comment.as_ref().and_then(|(_, e)| non_empty(e)),
            _ => None,
        }
    }

    /// Monaco `TimeBasedVariableResolver`.
    fn resolve_time(&self, name: &str) -> Option<String> {
        if !name.starts_with("CURRENT_") {
            return None;
        }
        let now = self.now.unwrap_or_else(Local::now);
        Some(match name {
            "CURRENT_YEAR" => now.year().to_string(),
            "CURRENT_YEAR_SHORT" => {
                let year = now.year().to_string();
                year[year.len().saturating_sub(2)..].to_string()
            }
            "CURRENT_MONTH" => format!("{:02}", now.month()),
            "CURRENT_DATE" => format!("{:02}", now.day()),
            "CURRENT_HOUR" => format!("{:02}", now.hour()),
            "CURRENT_MINUTE" => format!("{:02}", now.minute()),
            "CURRENT_SECOND" => format!("{:02}", now.second()),
            "CURRENT_MILLISECOND" => format!("{:03}", now.timestamp_subsec_millis()),
            "CURRENT_DAY_NAME" => {
                DAY_NAMES[now.weekday().num_days_from_sunday() as usize].to_string()
            }
            "CURRENT_DAY_NAME_SHORT" => {
                DAY_NAMES[now.weekday().num_days_from_sunday() as usize][..3].to_string()
            }
            "CURRENT_MONTH_NAME" => MONTH_NAMES[now.month0() as usize].to_string(),
            "CURRENT_MONTH_NAME_SHORT" => MONTH_NAMES[now.month0() as usize][..3].to_string(),
            "CURRENT_SECONDS_UNIX" => now.timestamp().to_string(),
            "CURRENT_MILLISECONDS_UNIX" => now.timestamp_millis().to_string(),
            "CURRENT_TIMEZONE_OFFSET" => {
                // JavaScript's offset has the opposite sign of UTC's.
                let offset = now.offset().local_minus_utc() / 60;
                let sign = if offset < 0 { '-' } else { '+' };
                let (hours, minutes) = (offset.abs() / 60, offset.abs() % 60);
                format!("{sign}{hours:02}:{minutes:02}")
            }
            "CURRENT_TIMEZONE_NAME" => iana_time_zone::get_timezone().ok()?,
            _ => return None,
        })
    }

    /// Monaco `WorkspaceBasedVariableResolver`.
    fn resolve_workspace(&self, name: &str) -> Option<String> {
        let workspace = self.workspace.as_ref()?;
        match name {
            "WORKSPACE_NAME" => Some(workspace.name.clone()),
            "WORKSPACE_FOLDER" => Some(workspace.folder.clone()),
            _ => None,
        }
    }
}

/// Monaco `RandomBasedVariableResolver`: six random decimal or hex digits,
/// or a v4 UUID.
fn resolve_random(name: &str) -> Option<String> {
    match name {
        "RANDOM" => Some(format!("{:06}", rand::random::<u32>() % 1_000_000)),
        "RANDOM_HEX" => Some(format!("{:06x}", rand::random::<u32>() & 0xff_ffff)),
        "UUID" => Some(uuid::Uuid::new_v4().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn context() -> SnippetVariableContext {
        SnippetVariableContext {
            path: Some("/home/user/project/src/main.rs".into()),
            workspace: Some(SnippetWorkspace {
                name: "project".into(),
                folder: "/home/user/project".into(),
            }),
            current_line: "    let word = 1;".into(),
            current_word: Some("word".into()),
            line_index: 4,
            cursor_index: 1,
            cursor_count: 2,
            ..Default::default()
        }
    }

    #[test]
    fn model_variables() {
        let cx = context();
        assert_eq!(cx.resolve("TM_FILENAME", None).as_deref(), Some("main.rs"));
        assert_eq!(
            cx.resolve("TM_FILENAME_BASE", None).as_deref(),
            Some("main")
        );
        assert_eq!(
            cx.resolve("TM_DIRECTORY", None).as_deref(),
            Some("/home/user/project/src")
        );
        assert_eq!(
            cx.resolve("TM_DIRECTORY_BASE", None).as_deref(),
            Some("src")
        );
        assert_eq!(
            cx.resolve("TM_FILEPATH", None).as_deref(),
            Some("/home/user/project/src/main.rs")
        );
        assert_eq!(
            cx.resolve("RELATIVE_FILEPATH", None).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(
            cx.resolve("WORKSPACE_NAME", None).as_deref(),
            Some("project")
        );
        // Monaco: a dotfile's base name is the whole name.
        let cx = SnippetVariableContext {
            path: Some("/a/.gitignore".into()),
            ..Default::default()
        };
        assert_eq!(
            cx.resolve("TM_FILENAME_BASE", None).as_deref(),
            Some(".gitignore")
        );
        // No file: no value.
        assert_eq!(
            SnippetVariableContext::default().resolve("TM_FILENAME", None),
            None
        );
    }

    #[test]
    fn selection_variables() {
        let mut cx = context();
        assert_eq!(
            cx.resolve("TM_CURRENT_LINE", None).as_deref(),
            Some("    let word = 1;")
        );
        assert_eq!(cx.resolve("TM_CURRENT_WORD", None).as_deref(), Some("word"));
        assert_eq!(cx.resolve("TM_LINE_INDEX", None).as_deref(), Some("4"));
        assert_eq!(cx.resolve("TM_LINE_NUMBER", None).as_deref(), Some("5"));
        assert_eq!(cx.resolve("CURSOR_INDEX", None).as_deref(), Some("1"));
        assert_eq!(cx.resolve("CURSOR_NUMBER", None).as_deref(), Some("2"));
        // No selection: no value.
        assert_eq!(cx.resolve("SELECTION", None), None);
        // A multi-line selection is indented like the variable.
        cx.selected_text = "a\nb\r\nc".into();
        cx.selection_is_multiline = true;
        cx.selection_line_leading_whitespace = "  ".into();
        assert_eq!(
            cx.resolve("TM_SELECTED_TEXT", Some("    ")).as_deref(),
            Some("a\n  b\r\n  c")
        );
    }

    #[test]
    fn clipboard_spreads_lines_over_cursors() {
        let mut cx = context();
        assert_eq!(cx.resolve("CLIPBOARD", None), None);
        cx.clipboard = Some("one\ntwo".into());
        assert_eq!(cx.resolve("CLIPBOARD", None).as_deref(), Some("one\ntwo"));
        cx.clipboard_spread = true;
        assert_eq!(cx.resolve("CLIPBOARD", None).as_deref(), Some("two"));
        cx.cursor_count = 3;
        assert_eq!(cx.resolve("CLIPBOARD", None).as_deref(), Some("one\ntwo"));
    }

    #[test]
    fn comment_variables() {
        let cx = SnippetVariableContext {
            line_comment: Some("//".into()),
            block_comment: Some(("/*".into(), "*/".into())),
            ..Default::default()
        };
        assert_eq!(cx.resolve("LINE_COMMENT", None).as_deref(), Some("//"));
        assert_eq!(
            cx.resolve("BLOCK_COMMENT_START", None).as_deref(),
            Some("/*")
        );
        assert_eq!(cx.resolve("BLOCK_COMMENT_END", None).as_deref(), Some("*/"));
        assert_eq!(
            SnippetVariableContext::default().resolve("LINE_COMMENT", None),
            None
        );
    }

    #[test]
    fn time_variables() {
        let now = Local.with_ymd_and_hms(2024, 3, 9, 7, 5, 3).unwrap();
        let cx = SnippetVariableContext {
            now: Some(now),
            ..Default::default()
        };
        let get = |name| cx.resolve(name, None).unwrap();
        assert_eq!(get("CURRENT_YEAR"), "2024");
        assert_eq!(get("CURRENT_YEAR_SHORT"), "24");
        assert_eq!(get("CURRENT_MONTH"), "03");
        assert_eq!(get("CURRENT_DATE"), "09");
        assert_eq!(get("CURRENT_HOUR"), "07");
        assert_eq!(get("CURRENT_MINUTE"), "05");
        assert_eq!(get("CURRENT_SECOND"), "03");
        assert_eq!(get("CURRENT_MILLISECOND"), "000");
        assert_eq!(get("CURRENT_DAY_NAME"), "Saturday");
        assert_eq!(get("CURRENT_DAY_NAME_SHORT"), "Sat");
        assert_eq!(get("CURRENT_MONTH_NAME"), "March");
        assert_eq!(get("CURRENT_MONTH_NAME_SHORT"), "Mar");
        assert_eq!(get("CURRENT_SECONDS_UNIX"), now.timestamp().to_string());
        let offset = get("CURRENT_TIMEZONE_OFFSET");
        assert!(
            regex::Regex::new(r"^[+-]\d\d:\d\d$")
                .unwrap()
                .is_match(&offset)
        );
    }

    #[test]
    fn random_variables() {
        let cx = SnippetVariableContext::default();
        let random = cx.resolve("RANDOM", None).unwrap();
        assert!(random.len() == 6 && random.chars().all(|c| c.is_ascii_digit()));
        let hex = cx.resolve("RANDOM_HEX", None).unwrap();
        assert!(hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()));
        let uuid = cx.resolve("UUID", None).unwrap();
        assert!(
            regex::Regex::new(
                r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
            )
            .unwrap()
            .is_match(&uuid)
        );
        assert_ne!(cx.resolve("UUID", None), Some(uuid));
        assert_eq!(cx.resolve("UNKNOWN", None), None);
    }
}
