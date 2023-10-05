use criterion::{criterion_group, criterion_main, Criterion};
use nix_uri::FlakeRef;

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("simple_uri", |b| {
        b.iter(|| TryInto::<FlakeRef>::try_into("github:a-kenji/nala"))
    });
    c.bench_function("simple_uri_gitlab", |b| {
        b.iter(|| TryInto::<FlakeRef>::try_into("gitlab:a-kenji/nala"))
    });
    c.bench_function("simple_uri_sourcehut", |b| {
        b.iter(|| TryInto::<FlakeRef>::try_into("sourcehut:a-kenji/nala"))
    });
    c.bench_function("simple_uri_with_params", |b| {
        b.iter(|| TryInto::<FlakeRef>::try_into("github:a-kenji/nala?dir=assets"))
    });
    c.bench_function("simple_uri_path", |b| {
        b.iter(|| TryInto::<FlakeRef>::try_into("path:/home/git/dev?dir=assets"))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
