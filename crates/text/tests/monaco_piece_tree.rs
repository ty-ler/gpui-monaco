//! Conformance: replays the fixed insert/delete sequences from Monaco's piece
//! tree tests against `Rope` and `Buffer`, checking content, lines and
//! coordinate conversions against a `String` reference after every step.

mod monaco_piece_tree_cases;

use monaco_piece_tree_cases::{CASES, Op};
use text::{Bias, Buffer, Point, Rope};

/// Monaco's `splitLines`: `\r\n`, `\r` and `\n` all end a line.
fn monaco_split_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_string)
        .collect()
}

fn check_rope(name: &str, rope: &Rope, reference: &str) {
    assert_eq!(rope.to_string(), reference, "{name}: content");
    if reference.contains('\r') {
        return;
    }
    let lines: Vec<&str> = reference.split('\n').collect();
    assert_eq!(
        rope.line_count() as usize,
        lines.len(),
        "{name}: line count"
    );
    let mut offset = 0;
    for (row, line) in lines.iter().enumerate() {
        let row = row as u32;
        assert_eq!(rope.line(row), *line, "{name}: line {row}");
        assert_eq!(
            rope.point_to_offset(Point::new(row, 0)),
            offset,
            "{name}: line start {row}"
        );
        for column in 0..=line.len() as u32 {
            let point = Point::new(row, column);
            assert_eq!(
                rope.offset_to_point(offset + column as usize),
                point,
                "{name}"
            );
        }
        offset += line.len() + 1;
    }
}

#[test]
fn monaco_piece_tree_sequences() {
    assert!(CASES.len() > 60, "expected the generated cases");
    for case in CASES {
        let mut rope = Rope::from(case.initial);
        let mut reference = case.initial.to_string();
        for op in case.ops {
            match *op {
                Op::Insert(offset, text) => {
                    rope.replace(offset..offset, text);
                    reference.insert_str(offset, text);
                }
                Op::Delete(offset, len) => {
                    // Monaco clamps deletions to the end of the text.
                    let end = (offset + len).min(reference.len());
                    let offset = offset.min(end);
                    let len = end - offset;
                    rope.replace(offset..offset + len, "");
                    reference.replace_range(offset..offset + len, "");
                }
            }
            check_rope(case.name, &rope, &reference);
        }

        // A model loaded from the final text has Monaco's lines.
        let buffer = Buffer::new(&reference);
        let expected = monaco_split_lines(&reference);
        let actual: Vec<String> = buffer.lines().collect();
        assert_eq!(actual, expected, "{}: model lines", case.name);
        for row in 0..buffer.line_count() {
            assert_eq!(
                buffer
                    .clip_point(Point::new(row, u32::MAX), Bias::Left)
                    .column,
                expected[row as usize].len() as u32
            );
        }
    }
}
