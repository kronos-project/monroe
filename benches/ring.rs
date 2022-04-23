#![feature(generic_associated_types, type_alias_impl_trait)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fmt;

mod monroe {
    pub mod ring;
}

mod actix {
    pub mod ring;
}

fn get_specs() -> Vec<RingSpec> {
    [(10, 100), (100, 1_000)]
        .into_iter()
        .map(|(nodes, rounds)| RingSpec::new(nodes, rounds))
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct RingSpec {
    /// The number of actors in the ring.
    pub nodes: u32,
    /// The amount of times the payload will be sent
    /// around the ring.
    pub rounds: u32,
    /// The limit is calculated by multiplying
    /// `rounds` and `nodes`.
    pub limit: u32,
}

impl RingSpec {
    fn new(nodes: u32, rounds: u32) -> Self {
        Self {
            nodes,
            rounds,
            limit: nodes * rounds,
        }
    }
}

impl fmt::Display for RingSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{} rounds through {} nodes", self.rounds, self.nodes)
    }
}

fn bench_monroe(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("monroe");

    for spec in get_specs() {
        let id = BenchmarkId::from_parameter(spec);
        group.bench_with_input(id, &spec, |b, spec| {
            b.to_async(&rt).iter(|| monroe::ring::run(*spec))
        });
    }
}

fn bench_actix(c: &mut Criterion) {
    let mut group = c.benchmark_group("actix");

    for spec in get_specs() {
        let id = BenchmarkId::from_parameter(spec);
        group.bench_with_input(id, &spec, |b, spec| b.iter(|| actix::ring::run(*spec)));
    }
}

criterion_group!(ring, bench_monroe, bench_actix);
criterion_main!(ring);
