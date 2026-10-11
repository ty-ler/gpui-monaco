//! Microsoft's codicon icon font, the icons Monaco's widgets use, for
//! gpui-monaco's optional `codicons` feature.
//!
//! `codicon.ttf` is from `@vscode/codicons` 0.0.46-43, © Microsoft
//! Corporation, licensed under CC-BY-4.0 (`LICENSE` and `NOTICE` in this
//! crate). Apps that ship it must keep that attribution. Its cmap also maps
//! `m` (`patch-codicon-font.py`), because GPUI's macOS text system skips
//! fonts without an `m` glyph.
#![no_std]

/// The font family name in [`CODICON_TTF`].
pub const CODICON_FONT_FAMILY: &str = "codicon";

/// `codicon.ttf` (CC-BY-4.0, © Microsoft Corporation).
pub static CODICON_TTF: &[u8] = include_bytes!("../codicon.ttf");
