//! Codicons: the [`Codicon`] table of Monaco's icon ids and code points
//! (`base/common/codiconsLibrary.ts` and the derived ids of
//! `base/common/codicons.ts`), `$(icon)` parsing and the [`icon`] element.
//!
//! The icon font itself, Microsoft's `@vscode/codicons` (© Microsoft,
//! CC-BY-4.0), is optional: the `codicons` cargo feature adds the
//! `gpui-monaco-codicons` crate, and [`crate::init`] (or the first icon
//! drawn) registers it with GPUI's text system. Apps that enable it must
//! ship its attribution (`NOTICE` in that crate). Without the feature, or
//! when the font can't be registered (as on GPUI's test platform), icons
//! are drawn as similar Unicode characters ([`Codicon::fallback`]).

use gpui::{
    App, Font, Global, Hsla, IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString,
    Styled as _, TextRun, Window, div, font, px,
};

/// The codicon font's family name.
pub const CODICON_FONT_FAMILY: &str = "codicon";

/// Monaco's default codicon size (`.codicon { font-size: 16px }`).
pub const DEFAULT_ICON_SIZE: Pixels = px(16.);

macro_rules! codicons {
    ($($variant:ident = $id:literal => $code_point:literal,)*) => {
        /// A Monaco codicon (`Codicon` in `base/common/codicons.ts`). Variant
        /// names are Monaco's keys; [`Codicon::id`] is the `codicon-<id>` class
        /// name and the `$(<id>)` theme icon syntax.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum Codicon {
            $(
                #[doc = concat!("`", $id, "`")]
                $variant,
            )*
        }

        impl Codicon {
            /// Every codicon, in Monaco's order.
            pub const ALL: &'static [Codicon] = &[$(Codicon::$variant,)*];

            /// Monaco's icon id (`ThemeIcon.id`), such as `"symbol-method"`.
            pub const fn id(self) -> &'static str {
                match self {
                    $(Codicon::$variant => $id,)*
                }
            }

            /// The icon's code point in the codicon font.
            pub const fn code_point(self) -> u32 {
                match self {
                    $(Codicon::$variant => $code_point,)*
                }
            }
        }
    };
}

impl Codicon {
    /// The icon's character in the codicon font.
    pub fn char(self) -> char {
        char::from_u32(self.code_point()).unwrap_or('\u{fffd}')
    }

    /// [`Self::char`] as a string, shared rather than allocated per use.
    pub(crate) fn text(self) -> SharedString {
        static TEXTS: std::sync::LazyLock<Vec<SharedString>> = std::sync::LazyLock::new(|| {
            Codicon::ALL
                .iter()
                .map(|icon| icon.char().to_string().into())
                .collect()
        });
        TEXTS[self as usize].clone()
    }

    /// The icon with Monaco id `id` (`$(id)` in labels and markdown), or
    /// `None` when there is no such codicon.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|icon| icon.id() == id)
    }

    /// A Unicode stand-in, used when the codicon font isn't available.
    pub fn fallback(self) -> &'static str {
        use Codicon::*;
        match self {
            Close | DialogClose | TreeFilterClear => "×",
            ChevronRight | MenuSubmenu => "›",
            ChevronLeft => "‹",
            ChevronDown | TreeItemExpanded => "⌄",
            ChevronUp => "⌃",
            TriangleRight => "▸",
            TriangleDown => "▾",
            ArrowUp => "↑",
            ArrowDown => "↓",
            ArrowLeft => "←",
            ArrowRight => "→",
            ArrowBoth => "↔",
            Add | Plus => "+",
            Remove => "−",
            Check | MenuSelection => "✓",
            Copy => "⧉",
            Ellipsis | More | ToolBarMore => "⋯",
            Lightbulb
            | LightBulb
            | LightbulbAutofix
            | LightbulbSparkle
            | LightbulbSparkleAutofix => "💡",
            Sparkle | SparkleFilled => "✨",
            StopCircle => "⏹",
            Error | DialogError => "⊗",
            Warning | DialogWarning => "⚠",
            Info | DialogInfo => "ⓘ",
            CaseSensitive => "Aa",
            WholeWord => "ab",
            Regex => ".*",
            PreserveCase => "AB",
            Selection => "≡",
            Replace => "⇄",
            ReplaceAll => "⇉",
            KeyboardTab => "⇥",
            KeyboardTabAbove => "⤒",
            KeyboardTabBelow => "⤓",
            Discard => "↺",
            Fold | Unfold => "↕",
            Wrench => "🔧",
            SymbolMethod | SymbolFunction | SymbolConstructor | SymbolOperator => "ƒ",
            SymbolField | SymbolVariable | SymbolProperty | SymbolValue | SymbolReference
            | SymbolTypeParameter | SymbolKey => "𝑥",
            SymbolClass | SymbolStruct | SymbolInterface | SymbolEnum | SymbolModule
            | SymbolObject | SymbolNamespace | SymbolPackage => "◆",
            SymbolEnumMember | SymbolConstant | SymbolNumber | SymbolBoolean | SymbolNull => "π",
            SymbolKeyword => "⌘",
            SymbolSnippet | SymbolArray => "{}",
            SymbolFile | SymbolFolder => "▤",
            SymbolText | SymbolString => "ab",
            SymbolColor | SymbolCustomColor => "◐",
            SymbolEvent => "ϟ",
            SymbolUnit => "u",
            Account => "@",
            Issues => "◎",
            Tools => "⚒",
            _ => "•",
        }
    }
}

/// Whether the codicon font is registered with this app's text system.
struct CodiconFont {
    available: bool,
}

impl Global for CodiconFont {}

/// Registers the codicon font with GPUI's text system, once per app, when
/// the `codicons` feature is on. Returns whether the font is available
/// (always `false` without the feature). Called by [`crate::init`], and
/// lazily by [`icon`].
pub fn register_codicon_font(cx: &mut App) -> bool {
    if let Some(state) = cx.try_global::<CodiconFont>() {
        return state.available;
    }
    let available = load_font(cx);
    cx.set_global(CodiconFont { available });
    available
}

#[cfg(feature = "codicons")]
fn load_font(cx: &App) -> bool {
    debug_assert_eq!(
        gpui_monaco_codicons::CODICON_FONT_FAMILY,
        CODICON_FONT_FAMILY
    );
    let text_system = cx.text_system();
    let font = std::borrow::Cow::Borrowed(gpui_monaco_codicons::CODICON_TTF);
    match text_system.add_fonts(vec![font]) {
        Ok(()) => {
            let available = text_system
                .all_font_names()
                .iter()
                .any(|name| name == CODICON_FONT_FAMILY);
            if available {
                // Load the face now, not on the first frame that draws an
                // icon (the suggest list while typing, say).
                text_system.prewarm_fonts(&[codicon_font()]);
            }
            available
        }
        Err(error) => {
            log::warn!("failed to load the codicon font: {error}");
            false
        }
    }
}

#[cfg(not(feature = "codicons"))]
fn load_font(_: &App) -> bool {
    false
}

/// Whether codicons render with the codicon font (else as
/// [`Codicon::fallback`] text). `false` until the font is registered.
pub fn codicons_available(cx: &App) -> bool {
    cx.try_global::<CodiconFont>()
        .is_some_and(|state| state.available)
}

/// The codicon font.
pub(crate) fn codicon_font() -> Font {
    font(CODICON_FONT_FAMILY)
}

/// How decoration text that may be codicons is drawn
/// ([`crate::DecorationOptions::glyph`] and `lines_decoration_text`): text
/// made only of codicon characters gets the codicon font, or its
/// characters' [`Codicon::fallback`]s when the font isn't available; other
/// text is drawn as is in the editor font (`None`).
pub(crate) fn decoration_text(text: &SharedString, cx: &App) -> (SharedString, Option<Font>) {
    // The codicon font's private use range.
    if text.is_empty()
        || !text
            .chars()
            .all(|ch| ('\u{ea60}'..='\u{ecff}').contains(&ch))
    {
        return (text.clone(), None);
    }
    if codicons_available(cx) {
        return (text.clone(), Some(codicon_font()));
    }
    // Without the font: each icon's stand-in (only looked up here).
    let fallback: Option<String> = text
        .chars()
        .map(|ch| {
            Codicon::ALL
                .iter()
                .find(|icon| icon.code_point() == ch as u32)
                .map(|icon| icon.fallback())
        })
        .collect();
    match fallback {
        Some(fallback) => (fallback.into(), None),
        None => (text.clone(), None),
    }
}

/// Shapes `icon` for painting at `size` (folding chevrons, the lightbulb),
/// or `None` when the codicon font isn't available.
pub(crate) fn shape_icon(
    icon: Codicon,
    size: Pixels,
    color: Hsla,
    window: &mut Window,
    cx: &App,
) -> Option<gpui::ShapedLine> {
    if !codicons_available(cx) {
        return None;
    }
    let text = icon.text();
    let run = TextRun {
        len: text.len(),
        font: codicon_font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    Some(window.text_system().shape_line(text, size, &[run], None))
}

/// A codicon element (Monaco's `<span class="codicon codicon-<id>">`): a
/// square box of the icon's size, centered, colored by the inherited text
/// color unless [`Icon::color`] is set.
#[derive(IntoElement)]
pub struct Icon {
    icon: Codicon,
    size: Pixels,
    color: Option<Hsla>,
}

/// A codicon element at Monaco's default 16px.
pub fn icon(icon: Codicon) -> Icon {
    Icon {
        icon,
        size: DEFAULT_ICON_SIZE,
        color: None,
    }
}

impl Icon {
    /// The icon's font size (and box size).
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// The icon's color.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let available = register_codicon_font(cx);
        let mut element = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .min_w(self.size)
            .h(self.size)
            .text_size(self.size)
            .line_height(self.size);
        if let Some(color) = self.color {
            element = element.text_color(color);
        }
        if available {
            // The whole font (not just the family), so every icon resolves
            // to the one face `register_codicon_font` prewarmed, whatever
            // features the surrounding text has.
            element.font(codicon_font()).child(self.icon.text())
        } else {
            // Unicode stand-ins are drawn a little smaller, like the
            // glyphs the widgets used before codicons.
            element
                .text_size(self.size * 0.8)
                .child(self.icon.fallback())
        }
    }
}

/// Splits `text` at Monaco's theme icon syntax (`$(name)` or
/// `$(name~modifier)`, `iconLabels.ts`): text segments and icons, in order.
/// `\$(name)` is an escaped icon and stays text (without the backslash).
/// Names that aren't codicons are kept as text, as Monaco renders an empty
/// span for unknown icons.
pub fn parse_label_with_icons(text: &str) -> Vec<LabelPart<'_>> {
    let mut parts = Vec::new();
    let mut text_start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$'
            && bytes.get(i + 1) == Some(&b'(')
            && let Some((name, end)) = icon_at(text, i)
        {
            let escaped = i > 0 && bytes[i - 1] == b'\\';
            if escaped {
                if text_start < i - 1 {
                    parts.push(LabelPart::Text(&text[text_start..i - 1]));
                }
                parts.push(LabelPart::Text(&text[i..end]));
            } else {
                if text_start < i {
                    parts.push(LabelPart::Text(&text[text_start..i]));
                }
                match Codicon::from_id(name) {
                    Some(icon) => parts.push(LabelPart::Icon(icon)),
                    None => parts.push(LabelPart::Text(&text[i..end])),
                }
            }
            text_start = end;
            i = end;
            continue;
        }
        i += 1;
    }
    if text_start < text.len() {
        parts.push(LabelPart::Text(&text[text_start..]));
    }
    parts
}

/// A segment of a label with icons ([`parse_label_with_icons`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelPart<'a> {
    /// Plain text.
    Text(&'a str),
    /// A `$(name)` codicon.
    Icon(Codicon),
}

/// The icon name and end offset of a `$(name)` or `$(name~modifier)` at
/// `start` (`ThemeIcon.iconNameExpression` and `iconModifierExpression`).
fn icon_at(text: &str, start: usize) -> Option<(&str, usize)> {
    let rest = &text[start + 2..];
    let name_len = rest
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-')
        .count();
    if name_len == 0 {
        return None;
    }
    let mut end = name_len;
    if rest.as_bytes().get(end) == Some(&b'~') {
        let modifier = rest[end + 1..]
            .bytes()
            .take_while(u8::is_ascii_alphabetic)
            .count();
        if modifier == 0 {
            return None;
        }
        end += 1 + modifier;
    }
    (rest.as_bytes().get(end) == Some(&b')')).then(|| (&rest[..name_len], start + 2 + end + 1))
}

/// Removes `$(name)` icons from `text`, with the space before or after
/// each (Monaco `stripIcons`, for accessible labels).
pub fn strip_icons(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for part in parse_label_with_icons(text) {
        match part {
            LabelPart::Text(text) => out.push_str(text),
            LabelPart::Icon(_) => {
                if out.ends_with(' ') {
                    out.pop();
                }
            }
        }
    }
    out
}

codicons! {
    Add = "add" => 0xea60,
    Plus = "plus" => 0xea60,
    GistNew = "gist-new" => 0xea60,
    RepoCreate = "repo-create" => 0xea60,
    Lightbulb = "lightbulb" => 0xea61,
    LightBulb = "light-bulb" => 0xea61,
    Repo = "repo" => 0xea62,
    RepoDelete = "repo-delete" => 0xea62,
    GistFork = "gist-fork" => 0xea63,
    RepoForked = "repo-forked" => 0xea63,
    GitPullRequest = "git-pull-request" => 0xea64,
    GitPullRequestAbandoned = "git-pull-request-abandoned" => 0xea64,
    RecordKeys = "record-keys" => 0xea65,
    Keyboard = "keyboard" => 0xea65,
    Tag = "tag" => 0xea66,
    GitPullRequestLabel = "git-pull-request-label" => 0xea66,
    TagAdd = "tag-add" => 0xea66,
    TagRemove = "tag-remove" => 0xea66,
    Person = "person" => 0xea67,
    PersonFollow = "person-follow" => 0xea67,
    PersonOutline = "person-outline" => 0xea67,
    PersonFilled = "person-filled" => 0xea67,
    SourceControl = "source-control" => 0xea68,
    Mirror = "mirror" => 0xea69,
    MirrorPublic = "mirror-public" => 0xea69,
    Star = "star" => 0xea6a,
    StarAdd = "star-add" => 0xea6a,
    StarDelete = "star-delete" => 0xea6a,
    StarEmpty = "star-empty" => 0xea6a,
    Comment = "comment" => 0xea6b,
    CommentAdd = "comment-add" => 0xea6b,
    Alert = "alert" => 0xea6c,
    Warning = "warning" => 0xea6c,
    Search = "search" => 0xea6d,
    SearchSave = "search-save" => 0xea6d,
    LogOut = "log-out" => 0xea6e,
    SignOut = "sign-out" => 0xea6e,
    LogIn = "log-in" => 0xea6f,
    SignIn = "sign-in" => 0xea6f,
    Eye = "eye" => 0xea70,
    EyeUnwatch = "eye-unwatch" => 0xea70,
    EyeWatch = "eye-watch" => 0xea70,
    CircleFilled = "circle-filled" => 0xea71,
    PrimitiveDot = "primitive-dot" => 0xea71,
    CloseDirty = "close-dirty" => 0xea71,
    DebugBreakpoint = "debug-breakpoint" => 0xea71,
    DebugBreakpointDisabled = "debug-breakpoint-disabled" => 0xea71,
    DebugHint = "debug-hint" => 0xea71,
    TerminalDecorationSuccess = "terminal-decoration-success" => 0xea71,
    PrimitiveSquare = "primitive-square" => 0xea72,
    Edit = "edit" => 0xea73,
    Pencil = "pencil" => 0xea73,
    Info = "info" => 0xea74,
    IssueOpened = "issue-opened" => 0xea74,
    GistPrivate = "gist-private" => 0xea75,
    GitForkPrivate = "git-fork-private" => 0xea75,
    Lock = "lock" => 0xea75,
    MirrorPrivate = "mirror-private" => 0xea75,
    Close = "close" => 0xea76,
    RemoveClose = "remove-close" => 0xea76,
    X = "x" => 0xea76,
    RepoSync = "repo-sync" => 0xea77,
    Sync = "sync" => 0xea77,
    Clone = "clone" => 0xea78,
    DesktopDownload = "desktop-download" => 0xea78,
    Beaker = "beaker" => 0xea79,
    Microscope = "microscope" => 0xea79,
    Vm = "vm" => 0xea7a,
    DeviceDesktop = "device-desktop" => 0xea7a,
    File = "file" => 0xea7b,
    More = "more" => 0xea7c,
    Ellipsis = "ellipsis" => 0xea7c,
    KebabHorizontal = "kebab-horizontal" => 0xea7c,
    MailReply = "mail-reply" => 0xea7d,
    Reply = "reply" => 0xea7d,
    Organization = "organization" => 0xea7e,
    OrganizationFilled = "organization-filled" => 0xea7e,
    OrganizationOutline = "organization-outline" => 0xea7e,
    NewFile = "new-file" => 0xea7f,
    FileAdd = "file-add" => 0xea7f,
    NewFolder = "new-folder" => 0xea80,
    FileDirectoryCreate = "file-directory-create" => 0xea80,
    Trash = "trash" => 0xea81,
    Trashcan = "trashcan" => 0xea81,
    History = "history" => 0xea82,
    Clock = "clock" => 0xea82,
    Folder = "folder" => 0xea83,
    FileDirectory = "file-directory" => 0xea83,
    SymbolFolder = "symbol-folder" => 0xea83,
    LogoGithub = "logo-github" => 0xea84,
    MarkGithub = "mark-github" => 0xea84,
    Github = "github" => 0xea84,
    Terminal = "terminal" => 0xea85,
    Console = "console" => 0xea85,
    Repl = "repl" => 0xea85,
    Zap = "zap" => 0xea86,
    SymbolEvent = "symbol-event" => 0xea86,
    Error = "error" => 0xea87,
    Stop = "stop" => 0xea87,
    Variable = "variable" => 0xea88,
    SymbolVariable = "symbol-variable" => 0xea88,
    Array = "array" => 0xea8a,
    SymbolArray = "symbol-array" => 0xea8a,
    SymbolModule = "symbol-module" => 0xea8b,
    SymbolPackage = "symbol-package" => 0xea8b,
    SymbolNamespace = "symbol-namespace" => 0xea8b,
    SymbolObject = "symbol-object" => 0xea8b,
    SymbolMethod = "symbol-method" => 0xea8c,
    SymbolFunction = "symbol-function" => 0xea8c,
    SymbolConstructor = "symbol-constructor" => 0xea8c,
    SymbolBoolean = "symbol-boolean" => 0xea8f,
    SymbolNull = "symbol-null" => 0xea8f,
    SymbolNumeric = "symbol-numeric" => 0xea90,
    SymbolNumber = "symbol-number" => 0xea90,
    SymbolStructure = "symbol-structure" => 0xea91,
    SymbolStruct = "symbol-struct" => 0xea91,
    SymbolParameter = "symbol-parameter" => 0xea92,
    SymbolTypeParameter = "symbol-type-parameter" => 0xea92,
    SymbolKey = "symbol-key" => 0xea93,
    SymbolText = "symbol-text" => 0xea93,
    SymbolReference = "symbol-reference" => 0xea94,
    GoToFile = "go-to-file" => 0xea94,
    SymbolEnum = "symbol-enum" => 0xea95,
    SymbolValue = "symbol-value" => 0xea95,
    SymbolRuler = "symbol-ruler" => 0xea96,
    SymbolUnit = "symbol-unit" => 0xea96,
    ActivateBreakpoints = "activate-breakpoints" => 0xea97,
    Archive = "archive" => 0xea98,
    ArrowBoth = "arrow-both" => 0xea99,
    ArrowDown = "arrow-down" => 0xea9a,
    ArrowLeft = "arrow-left" => 0xea9b,
    ArrowRight = "arrow-right" => 0xea9c,
    ArrowSmallDown = "arrow-small-down" => 0xea9d,
    ArrowSmallLeft = "arrow-small-left" => 0xea9e,
    ArrowSmallRight = "arrow-small-right" => 0xea9f,
    ArrowSmallUp = "arrow-small-up" => 0xeaa0,
    ArrowUp = "arrow-up" => 0xeaa1,
    Bell = "bell" => 0xeaa2,
    Bold = "bold" => 0xeaa3,
    Book = "book" => 0xeaa4,
    Bookmark = "bookmark" => 0xeaa5,
    DebugBreakpointConditionalUnverified = "debug-breakpoint-conditional-unverified" => 0xeaa6,
    DebugBreakpointConditional = "debug-breakpoint-conditional" => 0xeaa7,
    DebugBreakpointConditionalDisabled = "debug-breakpoint-conditional-disabled" => 0xeaa7,
    DebugBreakpointDataUnverified = "debug-breakpoint-data-unverified" => 0xeaa8,
    DebugBreakpointData = "debug-breakpoint-data" => 0xeaa9,
    DebugBreakpointDataDisabled = "debug-breakpoint-data-disabled" => 0xeaa9,
    DebugBreakpointLogUnverified = "debug-breakpoint-log-unverified" => 0xeaaa,
    DebugBreakpointLog = "debug-breakpoint-log" => 0xeaab,
    DebugBreakpointLogDisabled = "debug-breakpoint-log-disabled" => 0xeaab,
    Briefcase = "briefcase" => 0xeaac,
    Broadcast = "broadcast" => 0xeaad,
    Browser = "browser" => 0xeaae,
    Bug = "bug" => 0xeaaf,
    Calendar = "calendar" => 0xeab0,
    CaseSensitive = "case-sensitive" => 0xeab1,
    Check = "check" => 0xeab2,
    Checklist = "checklist" => 0xeab3,
    ChevronDown = "chevron-down" => 0xeab4,
    ChevronLeft = "chevron-left" => 0xeab5,
    ChevronRight = "chevron-right" => 0xeab6,
    ChevronUp = "chevron-up" => 0xeab7,
    ChromeClose = "chrome-close" => 0xeab8,
    ChromeMaximize = "chrome-maximize" => 0xeab9,
    ChromeMinimize = "chrome-minimize" => 0xeaba,
    ChromeRestore = "chrome-restore" => 0xeabb,
    CircleOutline = "circle-outline" => 0xeabc,
    Circle = "circle" => 0xeabc,
    DebugBreakpointUnverified = "debug-breakpoint-unverified" => 0xeabc,
    TerminalDecorationIncomplete = "terminal-decoration-incomplete" => 0xeabc,
    CircleSlash = "circle-slash" => 0xeabd,
    CircuitBoard = "circuit-board" => 0xeabe,
    ClearAll = "clear-all" => 0xeabf,
    Clippy = "clippy" => 0xeac0,
    CloseAll = "close-all" => 0xeac1,
    CloudDownload = "cloud-download" => 0xeac2,
    CloudUpload = "cloud-upload" => 0xeac3,
    Code = "code" => 0xeac4,
    CollapseAll = "collapse-all" => 0xeac5,
    ColorMode = "color-mode" => 0xeac6,
    CommentDiscussion = "comment-discussion" => 0xeac7,
    CreditCard = "credit-card" => 0xeac9,
    Dash = "dash" => 0xeacc,
    Dashboard = "dashboard" => 0xeacd,
    Database = "database" => 0xeace,
    DebugContinue = "debug-continue" => 0xeacf,
    DebugDisconnect = "debug-disconnect" => 0xead0,
    DebugPause = "debug-pause" => 0xead1,
    DebugRestart = "debug-restart" => 0xead2,
    DebugStart = "debug-start" => 0xead3,
    DebugStepInto = "debug-step-into" => 0xead4,
    DebugStepOut = "debug-step-out" => 0xead5,
    DebugStepOver = "debug-step-over" => 0xead6,
    DebugStop = "debug-stop" => 0xead7,
    Debug = "debug" => 0xead8,
    DeviceCameraVideo = "device-camera-video" => 0xead9,
    DeviceCamera = "device-camera" => 0xeada,
    DeviceMobile = "device-mobile" => 0xeadb,
    DiffAdded = "diff-added" => 0xeadc,
    DiffIgnored = "diff-ignored" => 0xeadd,
    DiffModified = "diff-modified" => 0xeade,
    DiffRemoved = "diff-removed" => 0xeadf,
    DiffRenamed = "diff-renamed" => 0xeae0,
    Diff = "diff" => 0xeae1,
    DiffSidebyside = "diff-sidebyside" => 0xeae1,
    Discard = "discard" => 0xeae2,
    EditorLayout = "editor-layout" => 0xeae3,
    EmptyWindow = "empty-window" => 0xeae4,
    Exclude = "exclude" => 0xeae5,
    Extensions = "extensions" => 0xeae6,
    EyeClosed = "eye-closed" => 0xeae7,
    FileBinary = "file-binary" => 0xeae8,
    FileCode = "file-code" => 0xeae9,
    FileMedia = "file-media" => 0xeaea,
    FilePdf = "file-pdf" => 0xeaeb,
    FileSubmodule = "file-submodule" => 0xeaec,
    FileSymlinkDirectory = "file-symlink-directory" => 0xeaed,
    FileSymlinkFile = "file-symlink-file" => 0xeaee,
    FileZip = "file-zip" => 0xeaef,
    Files = "files" => 0xeaf0,
    Filter = "filter" => 0xeaf1,
    Flame = "flame" => 0xeaf2,
    FoldDown = "fold-down" => 0xeaf3,
    FoldUp = "fold-up" => 0xeaf4,
    Fold = "fold" => 0xeaf5,
    FolderActive = "folder-active" => 0xeaf6,
    FolderOpened = "folder-opened" => 0xeaf7,
    Gear = "gear" => 0xeaf8,
    Gift = "gift" => 0xeaf9,
    GistSecret = "gist-secret" => 0xeafa,
    Gist = "gist" => 0xeafb,
    GitCommit = "git-commit" => 0xeafc,
    GitCompare = "git-compare" => 0xeafd,
    CompareChanges = "compare-changes" => 0xeafd,
    GitMerge = "git-merge" => 0xeafe,
    GithubAction = "github-action" => 0xeaff,
    GithubAlt = "github-alt" => 0xeb00,
    Globe = "globe" => 0xeb01,
    Grabber = "grabber" => 0xeb02,
    Graph = "graph" => 0xeb03,
    Gripper = "gripper" => 0xeb04,
    Heart = "heart" => 0xeb05,
    Home = "home" => 0xeb06,
    HorizontalRule = "horizontal-rule" => 0xeb07,
    Hubot = "hubot" => 0xeb08,
    Inbox = "inbox" => 0xeb09,
    IssueReopened = "issue-reopened" => 0xeb0b,
    Issues = "issues" => 0xeb0c,
    Italic = "italic" => 0xeb0d,
    Jersey = "jersey" => 0xeb0e,
    Json = "json" => 0xeb0f,
    Bracket = "bracket" => 0xeb0f,
    KebabVertical = "kebab-vertical" => 0xeb10,
    Key = "key" => 0xeb11,
    Law = "law" => 0xeb12,
    LightbulbAutofix = "lightbulb-autofix" => 0xeb13,
    LinkExternal = "link-external" => 0xeb14,
    Link = "link" => 0xeb15,
    ListOrdered = "list-ordered" => 0xeb16,
    ListUnordered = "list-unordered" => 0xeb17,
    LiveShare = "live-share" => 0xeb18,
    Loading = "loading" => 0xeb19,
    Location = "location" => 0xeb1a,
    MailRead = "mail-read" => 0xeb1b,
    Mail = "mail" => 0xeb1c,
    Markdown = "markdown" => 0xeb1d,
    Megaphone = "megaphone" => 0xeb1e,
    Mention = "mention" => 0xeb1f,
    Milestone = "milestone" => 0xeb20,
    GitPullRequestMilestone = "git-pull-request-milestone" => 0xeb20,
    MortarBoard = "mortar-board" => 0xeb21,
    Move = "move" => 0xeb22,
    MultipleWindows = "multiple-windows" => 0xeb23,
    Mute = "mute" => 0xeb24,
    NoNewline = "no-newline" => 0xeb25,
    Note = "note" => 0xeb26,
    Octoface = "octoface" => 0xeb27,
    OpenPreview = "open-preview" => 0xeb28,
    Package = "package" => 0xeb29,
    Paintcan = "paintcan" => 0xeb2a,
    Pin = "pin" => 0xeb2b,
    Play = "play" => 0xeb2c,
    Run = "run" => 0xeb2c,
    Plug = "plug" => 0xeb2d,
    PreserveCase = "preserve-case" => 0xeb2e,
    Preview = "preview" => 0xeb2f,
    Project = "project" => 0xeb30,
    Pulse = "pulse" => 0xeb31,
    Question = "question" => 0xeb32,
    Quote = "quote" => 0xeb33,
    RadioTower = "radio-tower" => 0xeb34,
    Reactions = "reactions" => 0xeb35,
    References = "references" => 0xeb36,
    Refresh = "refresh" => 0xeb37,
    Regex = "regex" => 0xeb38,
    RemoteExplorer = "remote-explorer" => 0xeb39,
    Remote = "remote" => 0xeb3a,
    Remove = "remove" => 0xeb3b,
    ReplaceAll = "replace-all" => 0xeb3c,
    Replace = "replace" => 0xeb3d,
    RepoClone = "repo-clone" => 0xeb3e,
    RepoForcePush = "repo-force-push" => 0xeb3f,
    RepoPull = "repo-pull" => 0xeb40,
    RepoPush = "repo-push" => 0xeb41,
    Report = "report" => 0xeb42,
    RequestChanges = "request-changes" => 0xeb43,
    Rocket = "rocket" => 0xeb44,
    RootFolderOpened = "root-folder-opened" => 0xeb45,
    RootFolder = "root-folder" => 0xeb46,
    Rss = "rss" => 0xeb47,
    Ruby = "ruby" => 0xeb48,
    SaveAll = "save-all" => 0xeb49,
    SaveAs = "save-as" => 0xeb4a,
    Save = "save" => 0xeb4b,
    ScreenFull = "screen-full" => 0xeb4c,
    ScreenNormal = "screen-normal" => 0xeb4d,
    SearchStop = "search-stop" => 0xeb4e,
    Server = "server" => 0xeb50,
    SettingsGear = "settings-gear" => 0xeb51,
    Settings = "settings" => 0xeb52,
    Shield = "shield" => 0xeb53,
    Smiley = "smiley" => 0xeb54,
    SortPrecedence = "sort-precedence" => 0xeb55,
    SplitHorizontal = "split-horizontal" => 0xeb56,
    SplitVertical = "split-vertical" => 0xeb57,
    Squirrel = "squirrel" => 0xeb58,
    StarFull = "star-full" => 0xeb59,
    StarHalf = "star-half" => 0xeb5a,
    SymbolClass = "symbol-class" => 0xeb5b,
    SymbolColor = "symbol-color" => 0xeb5c,
    SymbolConstant = "symbol-constant" => 0xeb5d,
    SymbolEnumMember = "symbol-enum-member" => 0xeb5e,
    SymbolField = "symbol-field" => 0xeb5f,
    SymbolFile = "symbol-file" => 0xeb60,
    SymbolInterface = "symbol-interface" => 0xeb61,
    SymbolKeyword = "symbol-keyword" => 0xeb62,
    SymbolMisc = "symbol-misc" => 0xeb63,
    SymbolOperator = "symbol-operator" => 0xeb64,
    SymbolProperty = "symbol-property" => 0xeb65,
    Wrench = "wrench" => 0xeb65,
    WrenchSubaction = "wrench-subaction" => 0xeb65,
    SymbolSnippet = "symbol-snippet" => 0xeb66,
    Tasklist = "tasklist" => 0xeb67,
    Telescope = "telescope" => 0xeb68,
    TextSize = "text-size" => 0xeb69,
    ThreeBars = "three-bars" => 0xeb6a,
    Thumbsdown = "thumbsdown" => 0xeb6b,
    Thumbsup = "thumbsup" => 0xeb6c,
    Tools = "tools" => 0xeb6d,
    TriangleDown = "triangle-down" => 0xeb6e,
    TriangleLeft = "triangle-left" => 0xeb6f,
    TriangleRight = "triangle-right" => 0xeb70,
    TriangleUp = "triangle-up" => 0xeb71,
    Twitter = "twitter" => 0xeb72,
    Unfold = "unfold" => 0xeb73,
    Unlock = "unlock" => 0xeb74,
    Unmute = "unmute" => 0xeb75,
    Unverified = "unverified" => 0xeb76,
    Verified = "verified" => 0xeb77,
    Versions = "versions" => 0xeb78,
    VmActive = "vm-active" => 0xeb79,
    VmOutline = "vm-outline" => 0xeb7a,
    VmRunning = "vm-running" => 0xeb7b,
    Watch = "watch" => 0xeb7c,
    Whitespace = "whitespace" => 0xeb7d,
    WholeWord = "whole-word" => 0xeb7e,
    Window = "window" => 0xeb7f,
    WordWrap = "word-wrap" => 0xeb80,
    ZoomIn = "zoom-in" => 0xeb81,
    ZoomOut = "zoom-out" => 0xeb82,
    ListFilter = "list-filter" => 0xeb83,
    ListFlat = "list-flat" => 0xeb84,
    ListSelection = "list-selection" => 0xeb85,
    Selection = "selection" => 0xeb85,
    ListTree = "list-tree" => 0xeb86,
    DebugBreakpointFunctionUnverified = "debug-breakpoint-function-unverified" => 0xeb87,
    DebugBreakpointFunction = "debug-breakpoint-function" => 0xeb88,
    DebugBreakpointFunctionDisabled = "debug-breakpoint-function-disabled" => 0xeb88,
    DebugStackframeActive = "debug-stackframe-active" => 0xeb89,
    CircleSmallFilled = "circle-small-filled" => 0xeb8a,
    DebugStackframeDot = "debug-stackframe-dot" => 0xeb8a,
    TerminalDecorationMark = "terminal-decoration-mark" => 0xeb8a,
    DebugStackframe = "debug-stackframe" => 0xeb8b,
    DebugStackframeFocused = "debug-stackframe-focused" => 0xeb8b,
    DebugBreakpointUnsupported = "debug-breakpoint-unsupported" => 0xeb8c,
    SymbolString = "symbol-string" => 0xeb8d,
    DebugReverseContinue = "debug-reverse-continue" => 0xeb8e,
    DebugStepBack = "debug-step-back" => 0xeb8f,
    DebugRestartFrame = "debug-restart-frame" => 0xeb90,
    DebugAlt = "debug-alt" => 0xeb91,
    CallIncoming = "call-incoming" => 0xeb92,
    CallOutgoing = "call-outgoing" => 0xeb93,
    Menu = "menu" => 0xeb94,
    ExpandAll = "expand-all" => 0xeb95,
    Feedback = "feedback" => 0xeb96,
    GitPullRequestReviewer = "git-pull-request-reviewer" => 0xeb96,
    GroupByRefType = "group-by-ref-type" => 0xeb97,
    UngroupByRefType = "ungroup-by-ref-type" => 0xeb98,
    Account = "account" => 0xeb99,
    GitPullRequestAssignee = "git-pull-request-assignee" => 0xeb99,
    BellDot = "bell-dot" => 0xeb9a,
    DebugConsole = "debug-console" => 0xeb9b,
    Library = "library" => 0xeb9c,
    Output = "output" => 0xeb9d,
    RunAll = "run-all" => 0xeb9e,
    SyncIgnored = "sync-ignored" => 0xeb9f,
    Pinned = "pinned" => 0xeba0,
    GithubInverted = "github-inverted" => 0xeba1,
    ServerProcess = "server-process" => 0xeba2,
    ServerEnvironment = "server-environment" => 0xeba3,
    Pass = "pass" => 0xeba4,
    IssueClosed = "issue-closed" => 0xeba4,
    StopCircle = "stop-circle" => 0xeba5,
    PlayCircle = "play-circle" => 0xeba6,
    Record = "record" => 0xeba7,
    DebugAltSmall = "debug-alt-small" => 0xeba8,
    VmConnect = "vm-connect" => 0xeba9,
    Cloud = "cloud" => 0xebaa,
    Merge = "merge" => 0xebab,
    Export = "export" => 0xebac,
    GraphLeft = "graph-left" => 0xebad,
    Magnet = "magnet" => 0xebae,
    Notebook = "notebook" => 0xebaf,
    Redo = "redo" => 0xebb0,
    CheckAll = "check-all" => 0xebb1,
    PinnedDirty = "pinned-dirty" => 0xebb2,
    PassFilled = "pass-filled" => 0xebb3,
    CircleLargeFilled = "circle-large-filled" => 0xebb4,
    CircleLarge = "circle-large" => 0xebb5,
    CircleLargeOutline = "circle-large-outline" => 0xebb5,
    Combine = "combine" => 0xebb6,
    Gather = "gather" => 0xebb6,
    Table = "table" => 0xebb7,
    VariableGroup = "variable-group" => 0xebb8,
    TypeHierarchy = "type-hierarchy" => 0xebb9,
    TypeHierarchySub = "type-hierarchy-sub" => 0xebba,
    TypeHierarchySuper = "type-hierarchy-super" => 0xebbb,
    GitPullRequestCreate = "git-pull-request-create" => 0xebbc,
    RunAbove = "run-above" => 0xebbd,
    RunBelow = "run-below" => 0xebbe,
    NotebookTemplate = "notebook-template" => 0xebbf,
    DebugRerun = "debug-rerun" => 0xebc0,
    WorkspaceTrusted = "workspace-trusted" => 0xebc1,
    WorkspaceUntrusted = "workspace-untrusted" => 0xebc2,
    WorkspaceUnknown = "workspace-unknown" => 0xebc3,
    TerminalCmd = "terminal-cmd" => 0xebc4,
    TerminalDebian = "terminal-debian" => 0xebc5,
    TerminalLinux = "terminal-linux" => 0xebc6,
    TerminalPowershell = "terminal-powershell" => 0xebc7,
    TerminalTmux = "terminal-tmux" => 0xebc8,
    TerminalUbuntu = "terminal-ubuntu" => 0xebc9,
    TerminalBash = "terminal-bash" => 0xebca,
    ArrowSwap = "arrow-swap" => 0xebcb,
    Copy = "copy" => 0xebcc,
    PersonAdd = "person-add" => 0xebcd,
    FilterFilled = "filter-filled" => 0xebce,
    Wand = "wand" => 0xebcf,
    DebugLineByLine = "debug-line-by-line" => 0xebd0,
    Inspect = "inspect" => 0xebd1,
    Layers = "layers" => 0xebd2,
    LayersDot = "layers-dot" => 0xebd3,
    LayersActive = "layers-active" => 0xebd4,
    Compass = "compass" => 0xebd5,
    CompassDot = "compass-dot" => 0xebd6,
    CompassActive = "compass-active" => 0xebd7,
    Azure = "azure" => 0xebd8,
    IssueDraft = "issue-draft" => 0xebd9,
    GitPullRequestClosed = "git-pull-request-closed" => 0xebda,
    GitPullRequestDraft = "git-pull-request-draft" => 0xebdb,
    DebugAll = "debug-all" => 0xebdc,
    DebugCoverage = "debug-coverage" => 0xebdd,
    RunErrors = "run-errors" => 0xebde,
    FolderLibrary = "folder-library" => 0xebdf,
    DebugContinueSmall = "debug-continue-small" => 0xebe0,
    BeakerStop = "beaker-stop" => 0xebe1,
    GraphLine = "graph-line" => 0xebe2,
    GraphScatter = "graph-scatter" => 0xebe3,
    PieChart = "pie-chart" => 0xebe4,
    BracketDot = "bracket-dot" => 0xebe5,
    BracketError = "bracket-error" => 0xebe6,
    LockSmall = "lock-small" => 0xebe7,
    AzureDevops = "azure-devops" => 0xebe8,
    VerifiedFilled = "verified-filled" => 0xebe9,
    Newline = "newline" => 0xebea,
    Layout = "layout" => 0xebeb,
    LayoutActivitybarLeft = "layout-activitybar-left" => 0xebec,
    LayoutActivitybarRight = "layout-activitybar-right" => 0xebed,
    LayoutPanelLeft = "layout-panel-left" => 0xebee,
    LayoutPanelCenter = "layout-panel-center" => 0xebef,
    LayoutPanelJustify = "layout-panel-justify" => 0xebf0,
    LayoutPanelRight = "layout-panel-right" => 0xebf1,
    LayoutPanel = "layout-panel" => 0xebf2,
    LayoutSidebarLeft = "layout-sidebar-left" => 0xebf3,
    LayoutSidebarRight = "layout-sidebar-right" => 0xebf4,
    LayoutStatusbar = "layout-statusbar" => 0xebf5,
    LayoutMenubar = "layout-menubar" => 0xebf6,
    LayoutCentered = "layout-centered" => 0xebf7,
    Target = "target" => 0xebf8,
    Indent = "indent" => 0xebf9,
    RecordSmall = "record-small" => 0xebfa,
    ErrorSmall = "error-small" => 0xebfb,
    TerminalDecorationError = "terminal-decoration-error" => 0xebfb,
    ArrowCircleDown = "arrow-circle-down" => 0xebfc,
    ArrowCircleLeft = "arrow-circle-left" => 0xebfd,
    ArrowCircleRight = "arrow-circle-right" => 0xebfe,
    ArrowCircleUp = "arrow-circle-up" => 0xebff,
    LayoutSidebarRightOff = "layout-sidebar-right-off" => 0xec00,
    LayoutPanelOff = "layout-panel-off" => 0xec01,
    LayoutSidebarLeftOff = "layout-sidebar-left-off" => 0xec02,
    Blank = "blank" => 0xec03,
    HeartFilled = "heart-filled" => 0xec04,
    Map = "map" => 0xec05,
    MapHorizontal = "map-horizontal" => 0xec05,
    FoldHorizontal = "fold-horizontal" => 0xec05,
    MapFilled = "map-filled" => 0xec06,
    MapHorizontalFilled = "map-horizontal-filled" => 0xec06,
    FoldHorizontalFilled = "fold-horizontal-filled" => 0xec06,
    CircleSmall = "circle-small" => 0xec07,
    BellSlash = "bell-slash" => 0xec08,
    BellSlashDot = "bell-slash-dot" => 0xec09,
    CommentUnresolved = "comment-unresolved" => 0xec0a,
    GitPullRequestGoToChanges = "git-pull-request-go-to-changes" => 0xec0b,
    GitPullRequestNewChanges = "git-pull-request-new-changes" => 0xec0c,
    SearchFuzzy = "search-fuzzy" => 0xec0d,
    CommentDraft = "comment-draft" => 0xec0e,
    Send = "send" => 0xec0f,
    Sparkle = "sparkle" => 0xec10,
    Insert = "insert" => 0xec11,
    Mic = "mic" => 0xec12,
    ThumbsdownFilled = "thumbsdown-filled" => 0xec13,
    ThumbsupFilled = "thumbsup-filled" => 0xec14,
    Coffee = "coffee" => 0xec15,
    Snake = "snake" => 0xec16,
    Game = "game" => 0xec17,
    Vr = "vr" => 0xec18,
    Chip = "chip" => 0xec19,
    Piano = "piano" => 0xec1a,
    Music = "music" => 0xec1b,
    MicFilled = "mic-filled" => 0xec1c,
    RepoFetch = "repo-fetch" => 0xec1d,
    Copilot = "copilot" => 0xec1e,
    LightbulbSparkle = "lightbulb-sparkle" => 0xec1f,
    Robot = "robot" => 0xec20,
    SparkleFilled = "sparkle-filled" => 0xec21,
    DiffSingle = "diff-single" => 0xec22,
    DiffMultiple = "diff-multiple" => 0xec23,
    SurroundWith = "surround-with" => 0xec24,
    Share = "share" => 0xec25,
    GitStash = "git-stash" => 0xec26,
    GitStashApply = "git-stash-apply" => 0xec27,
    GitStashPop = "git-stash-pop" => 0xec28,
    Vscode = "vscode" => 0xec29,
    VscodeInsiders = "vscode-insiders" => 0xec2a,
    CodeOss = "code-oss" => 0xec2b,
    RunCoverage = "run-coverage" => 0xec2c,
    RunAllCoverage = "run-all-coverage" => 0xec2d,
    Coverage = "coverage" => 0xec2e,
    GithubProject = "github-project" => 0xec2f,
    MapVertical = "map-vertical" => 0xec30,
    FoldVertical = "fold-vertical" => 0xec30,
    MapVerticalFilled = "map-vertical-filled" => 0xec31,
    FoldVerticalFilled = "fold-vertical-filled" => 0xec31,
    GoToSearch = "go-to-search" => 0xec32,
    Percentage = "percentage" => 0xec33,
    SortPercentage = "sort-percentage" => 0xec33,
    Attach = "attach" => 0xec34,
    GoToEditingSession = "go-to-editing-session" => 0xec35,
    EditSession = "edit-session" => 0xec36,
    CodeReview = "code-review" => 0xec37,
    CopilotWarning = "copilot-warning" => 0xec38,
    Python = "python" => 0xec39,
    CopilotLarge = "copilot-large" => 0xec3a,
    CopilotWarningLarge = "copilot-warning-large" => 0xec3b,
    KeyboardTab = "keyboard-tab" => 0xec3c,
    CopilotBlocked = "copilot-blocked" => 0xec3d,
    CopilotNotConnected = "copilot-not-connected" => 0xec3e,
    Flag = "flag" => 0xec3f,
    LightbulbEmpty = "lightbulb-empty" => 0xec40,
    SymbolMethodArrow = "symbol-method-arrow" => 0xec41,
    CopilotUnavailable = "copilot-unavailable" => 0xec42,
    RepoPinned = "repo-pinned" => 0xec43,
    KeyboardTabAbove = "keyboard-tab-above" => 0xec44,
    KeyboardTabBelow = "keyboard-tab-below" => 0xec45,
    GitPullRequestDone = "git-pull-request-done" => 0xec46,
    Mcp = "mcp" => 0xec47,
    ExtensionsLarge = "extensions-large" => 0xec48,
    LayoutPanelDock = "layout-panel-dock" => 0xec49,
    LayoutSidebarLeftDock = "layout-sidebar-left-dock" => 0xec4a,
    LayoutSidebarRightDock = "layout-sidebar-right-dock" => 0xec4b,
    CopilotInProgress = "copilot-in-progress" => 0xec4c,
    CopilotError = "copilot-error" => 0xec4d,
    CopilotSuccess = "copilot-success" => 0xec4e,
    ChatSparkle = "chat-sparkle" => 0xec4f,
    SearchSparkle = "search-sparkle" => 0xec50,
    EditSparkle = "edit-sparkle" => 0xec51,
    CopilotSnooze = "copilot-snooze" => 0xec52,
    SendToRemoteAgent = "send-to-remote-agent" => 0xec53,
    CommentDiscussionSparkle = "comment-discussion-sparkle" => 0xec54,
    ChatSparkleWarning = "chat-sparkle-warning" => 0xec55,
    ChatSparkleError = "chat-sparkle-error" => 0xec56,
    Collection = "collection" => 0xec57,
    NewCollection = "new-collection" => 0xec58,
    Thinking = "thinking" => 0xec59,
    Build = "build" => 0xec5a,
    CommentDiscussionQuote = "comment-discussion-quote" => 0xec5b,
    Cursor = "cursor" => 0xec5c,
    Eraser = "eraser" => 0xec5d,
    FileText = "file-text" => 0xec5e,
    Quotes = "quotes" => 0xec60,
    Rename = "rename" => 0xec61,
    RunWithDeps = "run-with-deps" => 0xec62,
    DebugConnected = "debug-connected" => 0xec63,
    Strikethrough = "strikethrough" => 0xec64,
    OpenInProduct = "open-in-product" => 0xec65,
    IndexZero = "index-zero" => 0xec66,
    Agent = "agent" => 0xec67,
    EditCode = "edit-code" => 0xec68,
    RepoSelected = "repo-selected" => 0xec69,
    Skip = "skip" => 0xec6a,
    MergeInto = "merge-into" => 0xec6b,
    GitBranchChanges = "git-branch-changes" => 0xec6c,
    GitBranchStagedChanges = "git-branch-staged-changes" => 0xec6d,
    GitBranchConflicts = "git-branch-conflicts" => 0xec6e,
    GitBranch = "git-branch" => 0xec6f,
    GitBranchCreate = "git-branch-create" => 0xec6f,
    GitBranchDelete = "git-branch-delete" => 0xec6f,
    SearchLarge = "search-large" => 0xec70,
    TerminalGitBash = "terminal-git-bash" => 0xec71,
    WindowActive = "window-active" => 0xec72,
    Forward = "forward" => 0xec73,
    Download = "download" => 0xec74,
    Clockface = "clockface" => 0xec75,
    Unarchive = "unarchive" => 0xec76,
    SessionInProgress = "session-in-progress" => 0xec77,
    CollectionSmall = "collection-small" => 0xec78,
    VmSmall = "vm-small" => 0xec79,
    CloudSmall = "cloud-small" => 0xec7a,
    AddSmall = "add-small" => 0xec7b,
    RemoveSmall = "remove-small" => 0xec7c,
    WorktreeSmall = "worktree-small" => 0xec7d,
    Worktree = "worktree" => 0xec7e,
    ScreenCut = "screen-cut" => 0xec7f,
    Ask = "ask" => 0xec80,
    Openai = "openai" => 0xec81,
    Claude = "claude" => 0xec82,
    OpenInWindow = "open-in-window" => 0xec83,
    NewSession = "new-session" => 0xec84,
    TerminalSecure = "terminal-secure" => 0xec85,
    ChatImport = "chat-import" => 0xec86,
    ChatExport = "chat-export" => 0xec87,
    ShareWindow = "share-window" => 0xec88,
    CircleSlashCompact = "circle-slash-compact" => 0xec89,
    CopilotCompact = "copilot-compact" => 0xec8a,
    FolderOpenedCompact = "folder-opened-compact" => 0xec8b,
    FolderCompact = "folder-compact" => 0xec8c,
    GearCompact = "gear-compact" => 0xec8d,
    GitBranchCompact = "git-branch-compact" => 0xec8e,
    LibraryCompact = "library-compact" => 0xec8f,
    RecordKeysCompact = "record-keys-compact" => 0xec90,
    RemoteCompact = "remote-compact" => 0xec91,
    RepoForkedCompact = "repo-forked-compact" => 0xec92,
    RepoCompact = "repo-compact" => 0xec93,
    ShieldCompact = "shield-compact" => 0xec94,
    SparkleCompact = "sparkle-compact" => 0xec95,
    SymbolColorCompact = "symbol-color-compact" => 0xec96,
    WindowCompact = "window-compact" => 0xec97,
    ErrorCompact = "error-compact" => 0xec98,
    WarningCompact = "warning-compact" => 0xec99,
    PassCompact = "pass-compact" => 0xec9a,
    Important = "important" => 0xec9b,
    ImportantCompact = "important-compact" => 0xec9c,
    RocketCompact = "rocket-compact" => 0xec9d,
    Unpin = "unpin" => 0xec9e,
    AddCompact = "add-compact" => 0xec9f,
    AttachCompact = "attach-compact" => 0xeca0,
    BeakerCompact = "beaker-compact" => 0xeca1,
    CheckCompact = "check-compact" => 0xeca2,
    ChecklistCompact = "checklist-compact" => 0xeca3,
    ChevronDownCompact = "chevron-down-compact" => 0xeca4,
    ChevronLeftCompact = "chevron-left-compact" => 0xeca5,
    ChevronRightCompact = "chevron-right-compact" => 0xeca6,
    ChevronUpCompact = "chevron-up-compact" => 0xeca7,
    CircleFilledCompact = "circle-filled-compact" => 0xeca8,
    CircleSmallFilledCompact = "circle-small-filled-compact" => 0xeca9,
    CloseCompact = "close-compact" => 0xecaa,
    CollapseAllCompact = "collapse-all-compact" => 0xecab,
    CommentCompact = "comment-compact" => 0xecac,
    CommentUnresolvedCompact = "comment-unresolved-compact" => 0xecad,
    DebugConnectedCompact = "debug-connected-compact" => 0xecae,
    DebugDisconnectCompact = "debug-disconnect-compact" => 0xecaf,
    EditCompact = "edit-compact" => 0xecb0,
    FileMediaCompact = "file-media-compact" => 0xecb1,
    GitFetch = "git-fetch" => 0xec1d,
    LightbulbCompact = "lightbulb-compact" => 0xecb3,
    LoadingCompact = "loading-compact" => 0xecb4,
    PassFilledCompact = "pass-filled-compact" => 0xecb5,
    ProjectCompact = "project-compact" => 0xecb6,
    RefreshCompact = "refresh-compact" => 0xecb7,
    SearchCompact = "search-compact" => 0xecb8,
    SessionInProgressCompact = "session-in-progress-compact" => 0xecb9,
    SyncCompact = "sync-compact" => 0xecba,
    TerminalCompact = "terminal-compact" => 0xecbb,
    VmPending = "vm-pending" => 0xecbc,
    WorktreeCompact = "worktree-compact" => 0xecbd,
    DeveloperTools = "developer-tools" => 0xecbe,
    CloudCompact = "cloud-compact" => 0xecbf,
    AgentCompact = "agent-compact" => 0xecc0,
    AskCompact = "ask-compact" => 0xecc1,
    SettingsCompact = "settings-compact" => 0xecc2,
    VmCompact = "vm-compact" => 0xecc3,
    RunCompact = "run-compact" => 0xecc4,
    GitPullRequestComment = "git-pull-request-comment" => 0xecc5,
    GitPullRequestError = "git-pull-request-error" => 0xecc6,
    RightPanelHide = "right-panel-hide" => 0xecc7,
    RightPanelShow = "right-panel-show" => 0xecc8,
    VscodeInsidersOutline = "vscode-insiders-outline" => 0xecc9,
    VscodeOutline = "vscode-outline" => 0xecca,
    VoiceMode = "voice-mode" => 0xeccb,
    VoiceModeCompact = "voice-mode-compact" => 0xeccc,
    MicDownload = "mic-download" => 0xeccd,
    MicDownloadCompact = "mic-download-compact" => 0xecce,
    VoiceModeDownload = "voice-mode-download" => 0xeccf,
    VoiceModeDownloadCompact = "voice-mode-download-compact" => 0xecd0,
    GoogleGemini = "google-gemini" => 0xecd1,
    Kimi = "kimi" => 0xecd2,
    Microsoft = "microsoft" => 0xecd3,
    Fish1Happy = "fish1-happy" => 0xecd4,
    Fish1Neutral = "fish1-neutral" => 0xecd5,
    Fish1Sad = "fish1-sad" => 0xecd6,
    Fish1VerySad = "fish1-very-sad" => 0xecd7,
    Fish2Happy = "fish2-happy" => 0xecd8,
    Fish2Neutral = "fish2-neutral" => 0xecd9,
    Fish2Sad = "fish2-sad" => 0xecda,
    Fish2VerySad = "fish2-very-sad" => 0xecdb,
    Fish3Happy = "fish3-happy" => 0xecdc,
    Fish3Neutral = "fish3-neutral" => 0xecdd,
    Fish3Sad = "fish3-sad" => 0xecde,
    Fish3VerySad = "fish3-very-sad" => 0xecdf,
    Fish4Happy = "fish4-happy" => 0xece0,
    Fish4Neutral = "fish4-neutral" => 0xece1,
    Fish4Sad = "fish4-sad" => 0xece2,
    Fish4VerySad = "fish4-very-sad" => 0xece3,
    PersonVoice = "person-voice" => 0xece4,
    PersonVoiceCompact = "person-voice-compact" => 0xece5,
    PersonVoiceFilled = "person-voice-filled" => 0xece6,
    PersonVoiceFilledCompact = "person-voice-filled-compact" => 0xece7,
    CloudDownloadCompact = "cloud-download-compact" => 0xece8,
    CloudUploadCompact = "cloud-upload-compact" => 0xece9,
    MicCompact = "mic-compact" => 0xecea,
    ArrowUpCompact = "arrow-up-compact" => 0xeceb,
    Xai = "xai" => 0xecec,
    ArrowCircleUpSparkle = "arrow-circle-up-sparkle" => 0xeced,
    CloseSmall = "close-small" => 0xecee,
    BookCompact = "book-compact" => 0xecef,
    MicOff = "mic-off" => 0xecf0,
    MicOffCompact = "mic-off-compact" => 0xecf1,
    CopilotDot = "copilot-dot" => 0xecf2,
    CopilotDotCompact = "copilot-dot-compact" => 0xecf3,
    LayoutDensityCompact = "layout-density-compact" => 0xecf4,
    LayoutDensityDefault = "layout-density-default" => 0xecf5,
    GripperCorner = "gripper-corner" => 0xecf6,
    MeteredConnection = "metered-connection" => 0xecf7,
    DialogError = "dialog-error" => 0xea87,
    DialogWarning = "dialog-warning" => 0xea6c,
    DialogInfo = "dialog-info" => 0xea74,
    DialogClose = "dialog-close" => 0xea76,
    TreeItemExpanded = "tree-item-expanded" => 0xeab4,
    TreeFilterOnTypeOn = "tree-filter-on-type-on" => 0xeb83,
    TreeFilterOnTypeOff = "tree-filter-on-type-off" => 0xeb85,
    TreeFilterClear = "tree-filter-clear" => 0xea76,
    TreeItemLoading = "tree-item-loading" => 0xeb19,
    MenuSelection = "menu-selection" => 0xeab2,
    MenuSubmenu = "menu-submenu" => 0xeab6,
    MenuBarMore = "menubar-more" => 0xea7c,
    ScrollbarButtonLeft = "scrollbar-button-left" => 0xeb6f,
    ScrollbarButtonRight = "scrollbar-button-right" => 0xeb70,
    ScrollbarButtonUp = "scrollbar-button-up" => 0xeb71,
    ScrollbarButtonDown = "scrollbar-button-down" => 0xeb6e,
    ToolBarMore = "toolbar-more" => 0xea7c,
    QuickInputBack = "quick-input-back" => 0xea9b,
    DropDownButton = "drop-down-button" => 0xeab4,
    SymbolCustomColor = "symbol-customcolor" => 0xeb5c,
    WorkspaceUnspecified = "workspace-unspecified" => 0xebc3,
    LightbulbSparkleAutofix = "lightbulb-sparkle-autofix" => 0xec1f,
    DebugBreakpointPending = "debug-breakpoint-pending" => 0xebd9,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_monaco() {
        // The ids of `codiconsLibrary.ts` plus `codicons.ts`'s derived ids.
        assert_eq!(Codicon::ALL.len(), 785);
        assert_eq!(Codicon::Add.id(), "add");
        assert_eq!(Codicon::Add.code_point(), 0xea60);
        assert_eq!(Codicon::Lightbulb.code_point(), 0xea61);
        assert_eq!(Codicon::SymbolMethod.id(), "symbol-method");
        assert_eq!(Codicon::SymbolMethod.code_point(), 0xea8c);
        assert_eq!(Codicon::Close.code_point(), 0xea76);
        assert_eq!(Codicon::ChevronRight.code_point(), 0xeab6);
        // Derived ids resolve to the icon they alias.
        assert_eq!(
            Codicon::TreeItemExpanded.code_point(),
            Codicon::ChevronDown.code_point()
        );
        assert_eq!(
            Codicon::MenuSubmenu.code_point(),
            Codicon::ChevronRight.code_point()
        );
        assert_eq!(
            Codicon::ToolBarMore.code_point(),
            Codicon::More.code_point()
        );
        // A derived id overrides the library's code point (`gitFetch`).
        assert_eq!(Codicon::GitFetch.code_point(), 0xec1d);
    }

    #[test]
    fn ids_are_unique_and_code_points_in_the_font() {
        let mut ids = std::collections::HashSet::new();
        for icon in Codicon::ALL {
            assert!(ids.insert(icon.id()), "duplicate id {}", icon.id());
            assert_eq!(icon.text().as_ref(), icon.char().to_string());
            assert_eq!(
                Codicon::from_id(icon.id()).map(Codicon::code_point),
                Some(icon.code_point())
            );
            // The private use range the font's cmap covers.
            assert!(
                (0xea60..=0xecf7).contains(&icon.code_point()),
                "{}",
                icon.id()
            );
        }
        assert_eq!(Codicon::from_id("no-such-icon"), None);
    }

    #[cfg(feature = "codicons")]
    #[test]
    fn bundled_font_is_codicon() {
        use gpui_monaco_codicons::CODICON_TTF;
        // A TrueType font whose name table names the `codicon` family.
        assert_eq!(&CODICON_TTF[..4], &[0, 1, 0, 0]);
        let name = CODICON_FONT_FAMILY.as_bytes();
        assert!(CODICON_TTF.windows(name.len()).any(|w| w == name));
    }

    #[cfg(not(feature = "codicons"))]
    #[gpui::test]
    fn without_the_feature_icons_are_unicode(cx: &mut gpui::TestAppContext) {
        assert!(!cx.update(register_codicon_font));
        assert!(!cx.update(|cx| codicons_available(cx)));
        cx.update(|cx| {
            let close: SharedString = Codicon::Close.char().to_string().into();
            assert_eq!(decoration_text(&close, cx), ("×".into(), None));
        });
        assert_eq!(Codicon::ChevronRight.fallback(), "›");
        assert_eq!(Codicon::SymbolMethod.fallback(), "ƒ");
    }

    #[test]
    fn theme_icon_syntax() {
        use LabelPart::*;
        assert_eq!(
            parse_label_with_icons("$(zap) Run $(debug-start)!"),
            [
                Icon(Codicon::Zap),
                Text(" Run "),
                Icon(Codicon::DebugStart),
                Text("!")
            ]
        );
        // Modifiers are accepted and ignored.
        assert_eq!(
            parse_label_with_icons("$(sync~spin)"),
            [Icon(Codicon::Sync)]
        );
        // Escaped icons stay text, without the backslash.
        assert_eq!(
            parse_label_with_icons(r"a \$(zap) b"),
            [Text("a "), Text("$(zap)"), Text(" b")]
        );
        // Unknown icons and malformed syntax stay text.
        assert_eq!(parse_label_with_icons("$(nope)"), [Text("$(nope)")]);
        assert_eq!(parse_label_with_icons("$(zap"), [Text("$(zap")]);
        assert_eq!(parse_label_with_icons("$()"), [Text("$()")]);
        assert_eq!(parse_label_with_icons("$(a~)"), [Text("$(a~)")]);
        assert_eq!(parse_label_with_icons("$"), [Text("$")]);
        assert_eq!(parse_label_with_icons(""), []);
    }

    #[gpui::test]
    fn icons_fall_back_without_the_font(cx: &mut gpui::TestAppContext) {
        let available = cx.update(register_codicon_font);
        assert_eq!(cx.update(|cx| codicons_available(cx)), available);
        cx.update(|cx| {
            let icon: SharedString = Codicon::Close.char().to_string().into();
            let (text, font) = decoration_text(&icon, cx);
            if available {
                assert_eq!(text, icon);
                assert_eq!(font.map(|f| f.family), Some(CODICON_FONT_FAMILY.into()));
            } else {
                assert_eq!(text.as_ref(), "×");
                assert_eq!(font, None);
            }
            // Other text is left alone.
            let (text, font) = decoration_text(&"+".into(), cx);
            assert_eq!((text.as_ref(), font), ("+", None));
        });
    }
}
