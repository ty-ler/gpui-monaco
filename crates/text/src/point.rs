use std::cmp::Ordering;
use std::ops::{Add, AddAssign, Sub};

/// A zero-based position in the text: `row` is the line index and `column` is
/// a **byte** offset within that line.
///
/// Monaco's `Position` is one-based with UTF-16 columns; use
/// [`PointUtf16::to_monaco`] / [`PointUtf16::from_monaco`] at API boundaries.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    /// Zero-based line index.
    pub row: u32,
    /// Byte offset within the line.
    pub column: u32,
}

impl Point {
    /// The largest representable point; resolves to the end of any text.
    pub const MAX: Self = Self {
        row: u32::MAX,
        column: u32::MAX,
    };

    /// Creates a point from a zero-based row and byte column.
    pub const fn new(row: u32, column: u32) -> Self {
        Self { row, column }
    }

    /// The start of the text.
    pub const fn zero() -> Self {
        Self::new(0, 0)
    }

    /// Whether this is the start of the text.
    pub fn is_zero(&self) -> bool {
        self.row == 0 && self.column == 0
    }
}

impl AddAssign for Point {
    /// Extends `self` by a text extent `rhs` (not a component-wise sum).
    fn add_assign(&mut self, rhs: Self) {
        if rhs.row == 0 {
            self.column += rhs.column;
        } else {
            self.row += rhs.row;
            self.column = rhs.column;
        }
    }
}

impl Add for Point {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self {
        self += rhs;
        self
    }
}

impl Sub for Point {
    type Output = Self;

    /// The extent from `rhs` to `self`. Panics if `rhs > self`.
    fn sub(self, rhs: Self) -> Self {
        debug_assert!(rhs <= self);
        if self.row == rhs.row {
            Self::new(0, self.column - rhs.column)
        } else {
            Self::new(self.row - rhs.row, self.column)
        }
    }
}

/// Like [`Point`], but `column` counts UTF-16 code units (Monaco's column unit).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointUtf16 {
    /// Zero-based line index.
    pub row: u32,
    /// Offset within the line in UTF-16 code units.
    pub column: u32,
}

impl PointUtf16 {
    /// Creates a point from a zero-based row and UTF-16 column.
    pub const fn new(row: u32, column: u32) -> Self {
        Self { row, column }
    }

    /// The start of the text.
    pub const fn zero() -> Self {
        Self::new(0, 0)
    }

    /// Monaco `(lineNumber, column)`, both one-based.
    pub fn to_monaco(self) -> (u32, u32) {
        (self.row + 1, self.column + 1)
    }

    /// From Monaco `(lineNumber, column)`. Like Monaco's `validatePosition`,
    /// a line number below 1 means the start of the text, and a column below 1
    /// means the start of the line.
    pub fn from_monaco(line_number: u32, column: u32) -> Self {
        if line_number < 1 {
            return Self::zero();
        }
        Self::new(line_number - 1, column.max(1) - 1)
    }
}

impl AddAssign for PointUtf16 {
    fn add_assign(&mut self, rhs: Self) {
        if rhs.row == 0 {
            self.column += rhs.column;
        } else {
            self.row += rhs.row;
            self.column = rhs.column;
        }
    }
}

/// An offset measured in UTF-16 code units from the start of the text.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OffsetUtf16(pub usize);

impl AddAssign for OffsetUtf16 {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

/// A position in every coordinate space at once. Returned by
/// [`Rope::locate`](crate::Rope::locate).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Location {
    /// Byte offset from the start of the text.
    pub offset: usize,
    /// Row and byte column.
    pub point: Point,
    /// Row and UTF-16 column.
    pub point_utf16: PointUtf16,
    /// Offset in UTF-16 code units from the start of the text.
    pub offset_utf16: OffsetUtf16,
    /// Number of chars (Unicode scalar values) from the start of the text.
    pub chars: usize,
}

impl Location {
    pub(crate) fn advance(&mut self, ch: char) {
        let utf8 = ch.len_utf8();
        let utf16 = ch.len_utf16();
        self.offset += utf8;
        self.offset_utf16.0 += utf16;
        self.chars += 1;
        if ch == '\n' {
            self.point.row += 1;
            self.point.column = 0;
            self.point_utf16.row += 1;
            self.point_utf16.column = 0;
        } else {
            self.point.column += utf8 as u32;
            self.point_utf16.column += utf16 as u32;
        }
    }
}

/// A coordinate that [`Rope::locate`](crate::Rope::locate) can seek to.
pub trait Target: Copy {
    #[doc(hidden)]
    fn cmp_location(&self, location: &Location) -> Ordering;
}

impl Target for usize {
    fn cmp_location(&self, location: &Location) -> Ordering {
        self.cmp(&location.offset)
    }
}

impl Target for Point {
    fn cmp_location(&self, location: &Location) -> Ordering {
        self.cmp(&location.point)
    }
}

impl Target for PointUtf16 {
    fn cmp_location(&self, location: &Location) -> Ordering {
        self.cmp(&location.point_utf16)
    }
}

impl Target for OffsetUtf16 {
    fn cmp_location(&self, location: &Location) -> Ordering {
        self.cmp(&location.offset_utf16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_extent_arithmetic() {
        assert_eq!(Point::new(1, 4) + Point::new(0, 3), Point::new(1, 7));
        assert_eq!(Point::new(1, 4) + Point::new(2, 3), Point::new(3, 3));
        assert_eq!(Point::new(3, 3) - Point::new(1, 4), Point::new(2, 3));
        assert_eq!(Point::new(1, 7) - Point::new(1, 4), Point::new(0, 3));
    }

    #[test]
    fn monaco_positions() {
        assert_eq!(PointUtf16::new(0, 0).to_monaco(), (1, 1));
        assert_eq!(PointUtf16::from_monaco(3, 5), PointUtf16::new(2, 4));
        assert_eq!(PointUtf16::from_monaco(0, 0), PointUtf16::new(0, 0));
        assert_eq!(PointUtf16::from_monaco(0, 7), PointUtf16::new(0, 0));
    }
}
