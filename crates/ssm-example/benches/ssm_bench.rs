//! Microbenchmarks for stactor (RwLock-based).
//!
//! Run with: cargo bench -p stactor-example

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;
use ssm::state_machine;

// =============================================================================
// State Machine (stactor with RwLock)
// =============================================================================

state_machine! {
    pub enum Counter {
        #[initial]
        Running { count: u64 },
    }

    impl Counter {
        fn increment(self: Running) -> Running {
            CounterStateData::Running { count: count + 1 }
        }

        fn get(self: Running) -> Running {
            // Read-only "transition" that returns to same state
            CounterStateData::Running { count }
        }
    }
}

// =============================================================================
// Benchmarks
// =============================================================================

/// Single-threaded write throughput
fn bench_single_thread_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_thread_writes");
    group.throughput(Throughput::Elements(1));

    group.bench_function("ssm", |b| {
        let counter = Counter::new();
        b.iter(|| {
            black_box(counter.increment().unwrap());
        });
    });

    group.finish();
}

/// Single-threaded read throughput
fn bench_single_thread_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_thread_reads");
    group.throughput(Throughput::Elements(1));

    group.bench_function("ssm", |b| {
        let counter = Counter::new();
        b.iter(|| {
            black_box(counter.get().unwrap());
        });
    });

    group.finish();
}

/// Concurrent writes from multiple threads
fn bench_concurrent_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_writes");

    for num_threads in [2, 4, 8] {
        let ops_per_thread = 1000;
        group.throughput(Throughput::Elements((num_threads * ops_per_thread) as u64));

        group.bench_with_input(
            BenchmarkId::new("ssm", num_threads),
            &num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let counter = Arc::new(Counter::new());
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let c = counter.clone();
                            std::thread::spawn(move || {
                                for _ in 0..ops_per_thread {
                                    c.increment().unwrap();
                                }
                            })
                        })
                        .collect();
                    for h in handles {
                        h.join().unwrap();
                    }
                    black_box(counter.get_state());
                });
            },
        );
    }

    group.finish();
}

/// Concurrent reads from multiple threads
fn bench_concurrent_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_reads");

    for num_threads in [2, 4, 8] {
        let ops_per_thread = 1000;
        group.throughput(Throughput::Elements((num_threads * ops_per_thread) as u64));

        group.bench_with_input(
            BenchmarkId::new("ssm", num_threads),
            &num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let counter = Arc::new(Counter::new());
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let c = counter.clone();
                            std::thread::spawn(move || {
                                for _ in 0..ops_per_thread {
                                    black_box(c.get_state());
                                }
                            })
                        })
                        .collect();
                    for h in handles {
                        h.join().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_single_thread_writes,
    bench_single_thread_reads,
    bench_concurrent_writes,
    bench_concurrent_reads,
);
criterion_main!(benches);
