//! `cargo bench -p gpui-monaco`
//!
//! Component benchmarks on a ~100k-line / ~3.5MB Rust file (the playground
//! sample repeated). The end-to-end budget lives in `tests/perf_budget.rs`
//! and `docs/PERFORMANCE.md`.

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use gpui_monaco::search::{SearchParams, find_matches};
use gpui_monaco::syntax::SyntaxSnapshot;
use gpui_monaco::text::{Buffer, Rope};
use gpui_monaco::view::folding::compute_indent_ranges;
use gpui_monaco::view::line_breaks::WrappingIndent;
use gpui_monaco::view::view_lines::{ViewLines, WrapSettings};

const LINES: usize = 100_000;

fn fixture() -> String {
    let sample = include_str!("../../../examples/playground/samples/sample.rs");
    let per_copy = sample.lines().count();
    sample.repeat(LINES.div_ceil(per_copy))
}

fn bench(c: &mut Criterion) {
    let text = fixture();
    let registry = gpui_monaco_languages::registry();
    let rust = registry.language_for_name("rust").expect("rust language");
    let buffer = Buffer::new(&text);
    let snapshot = buffer.snapshot();
    let rope: Rope = buffer.rope().clone();

    let mut group = c.benchmark_group("100k lines");
    group.sample_size(10);

    group.bench_function("syntax: full parse", |b| {
        b.iter(|| SyntaxSnapshot::parse(rust.clone(), black_box(&snapshot), &registry))
    });

    let parsed = SyntaxSnapshot::parse(rust.clone(), &snapshot, &registry);
    group.bench_function("syntax: reparse after one keystroke", |b| {
        b.iter_batched(
            || {
                let mut edited = buffer.clone();
                let middle = edited.len() / 2;
                let offset = edited.clip_offset(middle, text::Bias::Left);
                edited.edit([(offset..offset, "x")]).unwrap();
                let patch = edited.edits_since(buffer.version());
                (edited.snapshot(), patch)
            },
            |(edited, patch)| parsed.reparse(&rope, &edited, &patch, &registry),
            BatchSize::LargeInput,
        )
    });

    group.bench_function("syntax: highlights for one screen (60 lines)", |b| {
        let start = rope.point_to_offset(text::Point::new(50_000, 0));
        let end = rope.point_to_offset(text::Point::new(50_060, 0));
        b.iter(|| parsed.highlights(&rope, black_box(start..end)))
    });

    group.bench_function("view lines: new (no wrap)", |b| {
        b.iter(|| ViewLines::new(rope.clone(), buffer.version()))
    });

    group.bench_function("view lines: wrap at 80 columns", |b| {
        b.iter_batched(
            || ViewLines::new(rope.clone(), buffer.version()),
            |mut lines| {
                lines.set_wrap(Some(WrapSettings {
                    column: 80,
                    tab_size: 4,
                    indent: WrappingIndent::Same,
                    keep_all: false,
                }));
                lines
            },
            BatchSize::LargeInput,
        )
    });

    group.bench_function("folding: indent ranges", |b| {
        b.iter(|| compute_indent_ranges(black_box(&rope), 4, false, None))
    });

    let literal = SearchParams::new("self", false, false, None)
        .parse()
        .unwrap();
    group.bench_function("search: literal, all matches", |b| {
        b.iter(|| find_matches(&rope, &literal, 0..rope.len(), usize::MAX))
    });
    let regex = SearchParams::new(r"fn \w+\(", true, true, None)
        .parse()
        .unwrap();
    group.bench_function("search: regex, all matches", |b| {
        b.iter(|| find_matches(&rope, &regex, 0..rope.len(), usize::MAX))
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
