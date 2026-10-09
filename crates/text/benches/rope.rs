//! `cargo bench -p gpui-monaco-text`
//!
//! Fixture: ~10MB / ~170k lines of code-like text.

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use rand::{Rng, SeedableRng, rngs::StdRng};
use text::{Bias, Buffer, Point, Rope};

fn fixture() -> String {
    let line = "    let value = compute_something(argument_one, argument_two); // comment\n";
    line.repeat(10 * 1024 * 1024 / line.len())
}

fn bench(c: &mut Criterion) {
    let text = fixture();
    let rope = Rope::from(text.as_str());
    let rows = rope.max_point().row;

    c.bench_function("load 10MB rope", |b| {
        b.iter(|| Rope::from(black_box(text.as_str())))
    });
    c.bench_function("load 10MB buffer", |b| {
        b.iter(|| Buffer::new(black_box(text.as_str())))
    });

    c.bench_function("random single-char inserts (x100)", |b| {
        let mut rng = StdRng::seed_from_u64(1);
        b.iter_batched(
            || rope.clone(),
            |mut rope| {
                for _ in 0..100 {
                    let offset = rope.clip_offset(rng.random_range(0..rope.len()), Bias::Left);
                    rope.replace(offset..offset, "x");
                }
                rope
            },
            BatchSize::SmallInput,
        )
    });

    c.bench_function("buffer typing burst (x100, grouped)", |b| {
        let buffer = Buffer::new(&text);
        b.iter_batched(
            || buffer.clone(),
            |mut buffer| {
                let start = buffer.point_to_offset(Point::new(rows / 2, 4));
                for offset in start..start + 100 {
                    buffer.edit_grouped([(offset..offset, "a")], 1).unwrap();
                }
                buffer
            },
            BatchSize::SmallInput,
        )
    });

    c.bench_function("point_to_offset (x1000)", |b| {
        let mut rng = StdRng::seed_from_u64(2);
        b.iter(|| {
            for _ in 0..1000 {
                let row = rng.random_range(0..rows);
                black_box(rope.point_to_offset(Point::new(row, 10)));
            }
        })
    });

    c.bench_function("line(row) (x1000)", |b| {
        let mut rng = StdRng::seed_from_u64(3);
        b.iter(|| {
            for _ in 0..1000 {
                black_box(rope.line(rng.random_range(0..rows)));
            }
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
