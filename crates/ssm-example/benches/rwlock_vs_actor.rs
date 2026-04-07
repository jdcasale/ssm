//! Benchmark comparing RwLock vs Actor pattern under heavy write contention.
//!
//! Run with: cargo bench -p ssm-example --bench rwlock_vs_actor

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;
use tokio::sync::mpsc;

// =============================================================================
// RwLock-based counter (what ssm does now)
// =============================================================================

struct RwLockCounter {
    inner: std::sync::Arc<std::sync::RwLock<u64>>,
}

impl Clone for RwLockCounter {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl RwLockCounter {
    fn new() -> Self {
        Self { inner: Arc::new(std::sync::RwLock::new(0)) }
    }

    fn increment(&self) {
        let mut guard = self.inner.write().unwrap();
        *guard += 1;
    }

    fn get(&self) -> u64 {
        *self.inner.read().unwrap()
    }
}

// =============================================================================
// Actor-based counter (channel pattern)
// =============================================================================

enum ActorCommand {
    Increment { reply: tokio::sync::oneshot::Sender<()> },
    Get { reply: tokio::sync::oneshot::Sender<u64> },
}

struct ActorCounter {
    tx: mpsc::Sender<ActorCommand>,
}

impl Clone for ActorCounter {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone() }
    }
}

impl ActorCounter {
    fn new(buffer_size: usize) -> Self {
        let (tx, mut rx) = mpsc::channel::<ActorCommand>(buffer_size);

        tokio::spawn(async move {
            let mut count: u64 = 0;
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    ActorCommand::Increment { reply } => {
                        count += 1;
                        let _ = reply.send(());
                    }
                    ActorCommand::Get { reply } => {
                        let _ = reply.send(count);
                    }
                }
            }
        });

        Self { tx }
    }

    async fn increment(&self) {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx.send(ActorCommand::Increment { reply: reply_tx }).await.unwrap();
        reply_rx.await.unwrap();
    }

    async fn get(&self) -> u64 {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx.send(ActorCommand::Get { reply: reply_tx }).await.unwrap();
        reply_rx.await.unwrap()
    }
}

// =============================================================================
// Fire-and-forget actor (no reply, just send)
// =============================================================================

struct FireForgetActor {
    tx: mpsc::Sender<()>,
}

impl Clone for FireForgetActor {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone() }
    }
}

impl FireForgetActor {
    fn new(buffer_size: usize) -> (Self, tokio::task::JoinHandle<u64>) {
        let (tx, mut rx) = mpsc::channel::<()>(buffer_size);

        let handle = tokio::spawn(async move {
            let mut count: u64 = 0;
            while rx.recv().await.is_some() {
                count += 1;
            }
            count
        });

        (Self { tx }, handle)
    }

    async fn increment(&self) {
        let _ = self.tx.send(()).await;
    }
}

// =============================================================================
// Benchmarks
// =============================================================================

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap()
}

/// Heavy write contention - many writers, each doing many writes
fn bench_heavy_write_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("heavy_write_contention");

    for num_writers in [10, 50, 100, 500, 1000] {
        let ops_per_writer = 100;
        group.throughput(Throughput::Elements((num_writers * ops_per_writer) as u64));

        // RwLock
        group.bench_with_input(
            BenchmarkId::new("rwlock", num_writers),
            &num_writers,
            |b, &num_writers| {
                b.iter(|| {
                    let counter = RwLockCounter::new();
                    let handles: Vec<_> = (0..num_writers)
                        .map(|_| {
                            let c = counter.clone();
                            std::thread::spawn(move || {
                                for _ in 0..ops_per_writer {
                                    c.increment();
                                }
                            })
                        })
                        .collect();
                    for h in handles {
                        h.join().unwrap();
                    }
                    black_box(counter.get());
                });
            },
        );

        // Actor with reply (full round-trip)
        group.bench_with_input(
            BenchmarkId::new("actor_with_reply", num_writers),
            &num_writers,
            |b, &num_writers| {
                let rt = runtime();
                b.to_async(&rt).iter(|| async {
                    let counter = ActorCounter::new(num_writers * ops_per_writer);
                    let handles: Vec<_> = (0..num_writers)
                        .map(|_| {
                            let c = counter.clone();
                            tokio::spawn(async move {
                                for _ in 0..ops_per_writer {
                                    c.increment().await;
                                }
                            })
                        })
                        .collect();
                    for h in handles {
                        h.await.unwrap();
                    }
                    black_box(counter.get().await);
                });
            },
        );

        // Actor fire-and-forget (no reply, just buffer)
        group.bench_with_input(
            BenchmarkId::new("actor_fire_forget", num_writers),
            &num_writers,
            |b, &num_writers| {
                let rt = runtime();
                b.to_async(&rt).iter(|| async {
                    let (counter, actor_handle) = FireForgetActor::new(num_writers * ops_per_writer);
                    let handles: Vec<_> = (0..num_writers)
                        .map(|_| {
                            let c = counter.clone();
                            tokio::spawn(async move {
                                for _ in 0..ops_per_writer {
                                    c.increment().await;
                                }
                            })
                        })
                        .collect();
                    for h in handles {
                        h.await.unwrap();
                    }
                    drop(counter); // Close channel
                    black_box(actor_handle.await.unwrap());
                });
            },
        );
    }

    group.finish();
}

/// Sustained throughput - single measurement of ops/sec under contention
fn bench_sustained_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("sustained_throughput");

    let num_writers = 100;
    let duration_ms = 100;

    // RwLock
    group.bench_function("rwlock_100_writers", |b| {
        b.iter(|| {
            let counter = RwLockCounter::new();
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

            let handles: Vec<_> = (0..num_writers)
                .map(|_| {
                    let c = counter.clone();
                    let stop = stop.clone();
                    std::thread::spawn(move || {
                        let mut ops = 0u64;
                        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                            c.increment();
                            ops += 1;
                        }
                        ops
                    })
                })
                .collect();

            std::thread::sleep(std::time::Duration::from_millis(duration_ms));
            stop.store(true, std::sync::atomic::Ordering::Relaxed);

            let total_ops: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
            black_box(total_ops);
        });
    });

    // Actor fire-and-forget
    group.bench_function("actor_100_writers", |b| {
        let rt = runtime();
        b.to_async(&rt).iter(|| async {
            let (counter, actor_handle) = FireForgetActor::new(1_000_000);
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

            let handles: Vec<_> = (0..num_writers)
                .map(|_| {
                    let c = counter.clone();
                    let stop = stop.clone();
                    tokio::spawn(async move {
                        let mut ops = 0u64;
                        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                            c.increment().await;
                            ops += 1;
                        }
                        ops
                    })
                })
                .collect();

            tokio::time::sleep(tokio::time::Duration::from_millis(duration_ms)).await;
            stop.store(true, std::sync::atomic::Ordering::Relaxed);

            let total_ops: u64 = futures::future::join_all(handles)
                .await
                .into_iter()
                .map(|r| r.unwrap())
                .sum();

            drop(counter);
            let _ = actor_handle.await;
            black_box(total_ops);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_heavy_write_contention,
    bench_sustained_throughput,
);
criterion_main!(benches);
