//! HTML on the system clipboard, beside the text GPUI writes and reads.
//!
//! GPUI's clipboard (0.3.8) holds text, images and file paths, with no HTML,
//! so Monaco's `copyWithSyntaxHighlighting` (writing `text/html` when
//! copying) and paste providers' `text/html` (reading it) talk to the
//! platform clipboard here: `NSPasteboard` on macOS and the `HTML Format`
//! clipboard format on Windows. Elsewhere (Linux, where GPUI owns the
//! selection, and the browser) there's no HTML.
//!
//! Both only act when the system clipboard's text is the text GPUI wrote or
//! read, so they never touch the clipboard of a test platform's copy, and
//! never pair HTML with other text.

/// Whether this platform's clipboard takes HTML here.
pub(crate) const SUPPORTS_HTML: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Adds `html` to the system clipboard, if its text is `text` (GPUI just
/// wrote it). Whether it did.
pub(crate) fn add_html(text: &str, html: &str) -> bool {
    imp::add_html(text, html)
}

/// The system clipboard's HTML, if its text is `text` (what GPUI read).
#[allow(dead_code)] // For paste providers (#309).
pub(crate) fn read_html(text: &str) -> Option<String> {
    imp::read_html(text)
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeHTML, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    fn holds_text(pasteboard: &NSPasteboard, text: &str) -> bool {
        // SAFETY: AppKit's constant, read on any thread.
        let string = unsafe { pasteboard.stringForType(NSPasteboardTypeString) };
        string.is_some_and(|string| string.to_string() == text)
    }

    pub(super) fn add_html(text: &str, html: &str) -> bool {
        let pasteboard = NSPasteboard::generalPasteboard();
        if !holds_text(&pasteboard, text) {
            return false;
        }
        // Adds a type to the item GPUI wrote (it cleared the pasteboard).
        // SAFETY: as above.
        unsafe { pasteboard.setString_forType(&NSString::from_str(html), NSPasteboardTypeHTML) }
    }

    pub(super) fn read_html(text: &str) -> Option<String> {
        let pasteboard = NSPasteboard::generalPasteboard();
        if !holds_text(&pasteboard, text) {
            return None;
        }
        // SAFETY: as above.
        let html = unsafe { pasteboard.stringForType(NSPasteboardTypeHTML) }?;
        Some(html.to_string())
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
    };
    use windows::Win32::System::Ole::CF_UNICODETEXT;
    use windows::core::w;

    /// The clipboard, open until dropped.
    struct Open;

    impl Open {
        fn new() -> Option<Self> {
            // SAFETY: plain Win32 calls; closed on drop.
            unsafe { OpenClipboard(None) }.ok().map(|()| Self)
        }
    }

    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: opened in `new`.
            unsafe { CloseClipboard() }.ok();
        }
    }

    fn html_format() -> u32 {
        // SAFETY: a static wide string.
        unsafe { RegisterClipboardFormatW(w!("HTML Format")) }
    }

    /// The bytes of the clipboard's data in `format`.
    fn data(format: u32) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open; the handle is the clipboard's
        // global, locked while it's read.
        unsafe {
            let global = HGLOBAL(GetClipboardData(format).ok()?.0);
            let size = GlobalSize(global);
            let ptr = GlobalLock(global) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
            GlobalUnlock(global).ok();
            Some(bytes)
        }
    }

    /// Whether the clipboard's text is `text`, with NULs as spaces as GPUI
    /// writes them.
    fn holds_text(text: &str) -> bool {
        let Some(bytes) = data(u32::from(CF_UNICODETEXT.0)) else {
            return false;
        };
        let wide: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_ne_bytes(*pair))
            .take_while(|&unit| unit != 0)
            .collect();
        wide.iter()
            .copied()
            .eq(text.replace('\0', " ").encode_utf16())
    }

    pub(super) fn add_html(text: &str, html: &str) -> bool {
        let Some(_open) = Open::new() else {
            return false;
        };
        if !holds_text(text) {
            return false;
        }
        let mut bytes = super::cf_html(html).into_bytes();
        bytes.push(0);
        // SAFETY: a fresh global, filled and handed to the clipboard (which
        // then owns it), or freed if that fails.
        unsafe {
            let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
                return false;
            };
            let ptr = GlobalLock(global) as *mut u8;
            if ptr.is_null() {
                GlobalFree(Some(global)).ok();
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            GlobalUnlock(global).ok();
            // Added beside GPUI's formats (no `EmptyClipboard`).
            if SetClipboardData(html_format(), Some(HANDLE(global.0))).is_err() {
                GlobalFree(Some(global)).ok();
                return false;
            }
        }
        true
    }

    pub(super) fn read_html(text: &str) -> Option<String> {
        let _open = Open::new()?;
        if !holds_text(text) {
            return None;
        }
        let bytes = data(html_format())?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        super::html_from_cf_html(&String::from_utf8_lossy(&bytes[..end]))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    pub(super) fn add_html(_text: &str, _html: &str) -> bool {
        false
    }

    pub(super) fn read_html(_text: &str) -> Option<String> {
        None
    }
}

/// Windows' `HTML Format` (CF_HTML) for the fragment `html`: a header of
/// byte offsets, then the document, as Chromium writes it.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn cf_html(html: &str) -> String {
    const HEADER: &str = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\n\
        StartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    const PREFIX: &str = "<html>\r\n<body>\r\n<!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment-->\r\n</body>\r\n</html>";
    let start_html = HEADER.len();
    let start_fragment = start_html + PREFIX.len();
    let end_fragment = start_fragment + html.len();
    let end_html = end_fragment + SUFFIX.len();
    format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\n\
         StartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n\
         {PREFIX}{html}{SUFFIX}"
    )
}

/// The document in a CF_HTML string (from `StartHTML` to `EndHTML`, as
/// browsers give `text/html`), or what follows the header.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn html_from_cf_html(data: &str) -> Option<String> {
    let offset = |key: &str| -> Option<usize> {
        let at = data.find(key)? + key.len();
        let digits: String = data[at..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok()
    };
    if let (Some(start), Some(end)) = (offset("StartHTML:"), offset("EndHTML:"))
        && start <= end
        && let Some(html) = data.get(start..end.min(data.len()))
    {
        return Some(html.to_string());
    }
    let start = data.find('<')?;
    Some(data[start..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_html_offsets_point_at_the_fragment_and_document() {
        let html = "<div>é<b>x</b></div>";
        let data = cf_html(html);
        let offset = |key: &str| -> usize {
            let at = data.find(key).unwrap() + key.len();
            data[at..at + 10].parse().unwrap()
        };
        let (start, end) = (offset("StartFragment:"), offset("EndFragment:"));
        assert_eq!(&data[start..end], html);
        let (start, end) = (offset("StartHTML:"), offset("EndHTML:"));
        assert!(data[start..end].starts_with("<html>"));
        assert!(data[start..end].ends_with("</html>"));
        assert_eq!(end, data.len());
        assert_eq!(html_from_cf_html(&data).as_deref(), Some(&data[start..end]));
    }

    #[test]
    fn cf_html_without_offsets_gives_what_follows_the_header() {
        assert_eq!(
            html_from_cf_html("Version:0.9\r\n<b>x</b>").as_deref(),
            Some("<b>x</b>")
        );
    }
}
