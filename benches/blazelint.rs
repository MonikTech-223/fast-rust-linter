use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::path::Path;

fn dummy_analysis(path: &Path) {
    let _ = std::fs::read_to_string(path);
}

fn benchmark_blazelint(c: &mut Criterion) {
    let mut group = c.benchmark_group("blazelint");

    group.bench_function("analyze_10mb_file", |b| {
        b.iter(|| {
            dummy_analysis(black_box(Path::new("test_data/large_test.py")));
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_blazelint);
criterion_main!(benches);
