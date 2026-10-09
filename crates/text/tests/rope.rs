//! `Rope::for_each_line` against `str::split('\n')`.

use proptest::prelude::*;
use text::{Point, Rope};

proptest! {
    #[test]
    fn for_each_line_matches_split(
        text in "([a-zé😀 ]{0,300}\n){0,40}[a-z]{0,300}",
        first_row in 0u32..50,
        stop_after in 0usize..60,
    ) {
        let rope = Rope::from(text.as_str());
        let first_row = first_row.min(rope.max_point().row);
        let start = rope.point_to_offset(Point::new(first_row, 0));
        let mut expected = Vec::new();
        let mut offset = 0;
        for line in text.split('\n') {
            if offset >= start {
                expected.push((offset, line.to_string()));
            }
            offset += line.len() + 1;
        }
        expected.truncate(stop_after + 1);

        let mut actual = Vec::new();
        rope.for_each_line(start..rope.len(), |line_start, line| {
            actual.push((line_start, line.to_string()));
            actual.len() <= stop_after
        });
        prop_assert_eq!(actual, expected);
    }
}
