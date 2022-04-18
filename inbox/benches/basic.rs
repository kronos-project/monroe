use std::fmt::{self, Debug};

use async_trait::async_trait;
use criterion::{black_box, criterion_group, criterion_main, Bencher, BenchmarkId, Criterion};

#[async_trait]
trait Sender: Clone + Send + Sync + Sized + 'static {
    type Item: Debug + Default + Send;

    type Receiver: Receiver<Item = Self::Item>;

    fn unbounded() -> (Self, Self::Receiver);

    fn bounded(n: usize) -> (Self, Self::Receiver);

    async fn send(&self, msg: Self::Item);
}

#[async_trait]
trait Receiver: Send + Sync + Sized + 'static {
    type Item: Default;

    async fn recv(&self) -> Self::Item;

    async fn recv_no_panic(&self) -> Option<Self::Item>;
}

#[async_trait]
impl<T: Send + Debug + Default + 'static> Sender for monroe_inbox::Sender<T> {
    type Item = T;

    type Receiver = monroe_inbox::Receiver<T>;

    fn unbounded() -> (Self, Self::Receiver) {
        monroe_inbox::unbounded()
    }

    fn bounded(n: usize) -> (Self, Self::Receiver) {
        monroe_inbox::bounded(n)
    }

    async fn send(&self, msg: Self::Item) {
        self.send(msg).await.unwrap();
    }
}

#[async_trait]
impl<T: Send + Default + 'static> Receiver for monroe_inbox::Receiver<T> {
    type Item = T;

    async fn recv(&self) -> Self::Item {
        self.recv().await.unwrap()
    }

    async fn recv_no_panic(&self) -> Option<Self::Item> {
        self.recv().await.ok()
    }
}

#[async_trait]
impl<T: Send + Debug + Default + 'static> Sender for flume::Sender<T> {
    type Item = T;

    type Receiver = flume::Receiver<T>;

    fn unbounded() -> (Self, Self::Receiver) {
        flume::unbounded()
    }

    fn bounded(n: usize) -> (Self, Self::Receiver) {
        flume::bounded(n)
    }

    async fn send(&self, msg: Self::Item) {
        self.send_async(msg).await.unwrap();
    }
}

#[async_trait]
impl<T: Send + Default + 'static> Receiver for flume::Receiver<T> {
    type Item = T;

    async fn recv(&self) -> Self::Item {
        self.recv_async().await.unwrap()
    }

    async fn recv_no_panic(&self) -> Option<Self::Item> {
        self.recv_async().await.ok()
    }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn mt_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn test_create<S: Sender>(b: &mut Bencher) {
    b.iter(|| S::unbounded());
}

fn test_oneshot<S: Sender>(b: &mut Bencher) {
    b.to_async(rt()).iter(|| async {
        let (tx, rx) = S::unbounded();
        tx.send(Default::default()).await;
        black_box(rx.recv());
    });
}

fn test_inout<S: Sender>(b: &mut Bencher) {
    let (tx, rx) = S::unbounded();
    b.to_async(rt()).iter(|| async {
        tx.send(Default::default()).await;
        black_box(rx.recv());
    });
}

fn test_hydra<S: Sender>(b: &mut Bencher, thread_num: usize, msg_num: usize) {
    let rt = mt_rt();

    let (main_tx, main_rx) = S::unbounded();

    let mut txs = Vec::new();
    for _ in 0..thread_num {
        let main_tx = main_tx.clone();
        let (tx, rx) = S::unbounded();
        txs.push(tx);

        rt.spawn(async move {
            while let Some(msg) = rx.recv_no_panic().await {
                main_tx.send(msg).await;
            }
        });
    }

    drop(main_tx);

    b.to_async(&rt).iter(|| async {
        for tx in &txs {
            for _ in 0..msg_num {
                tx.send(Default::default()).await;
            }
        }

        for _ in 0..thread_num {
            for _ in 0..msg_num {
                black_box(main_rx.recv().await);
            }
        }
    });
}

fn test_robin_u<S: Sender>(b: &mut Bencher, thread_num: usize, msg_num: usize) {
    let rt = mt_rt();

    let (mut main_tx, main_rx) = S::unbounded();

    for _ in 0..thread_num {
        let (mut tx, rx) = S::unbounded();
        std::mem::swap(&mut tx, &mut main_tx);

        rt.spawn(async move {
            while let Some(msg) = rx.recv_no_panic().await {
                tx.send(msg).await;
            }
        });
    }

    b.to_async(&rt).iter(|| async {
        for _ in 0..msg_num {
            main_tx.send(Default::default()).await;
        }

        for _ in 0..msg_num {
            black_box(main_rx.recv().await);
        }
    });
}

fn test_robin_b<S: Sender>(b: &mut Bencher, thread_num: usize, msg_num: usize) {
    let rt = mt_rt();

    let (mut main_tx, main_rx) = S::bounded(1);

    for _ in 0..thread_num {
        let (mut tx, rx) = S::bounded(1);
        std::mem::swap(&mut tx, &mut main_tx);

        rt.spawn(async move {
            while let Some(msg) = rx.recv_no_panic().await {
                tx.send(msg).await;
            }
        });
    }

    b.to_async(&rt).iter(|| async {
        let main_tx = main_tx.clone();
        rt.spawn(async move {
            for _ in 0..msg_num {
                main_tx.send(Default::default()).await;
            }
        });

        for _ in 0..msg_num {
            black_box(main_rx.recv().await);
        }
    });
}

macro_rules! bench {
    ($name:ident: $fn:ident($($arg:expr),*)) => {
        fn $name(b: &mut Criterion) {
            let mut g = b.benchmark_group(stringify!($name));
            g.bench_function("flume", |b| {
                $fn::<flume::Sender<u32>>(b$(, $arg)*)
            });

            g.bench_function("monroe", |b| {
                $fn::<monroe_inbox::Sender<u32>>(b$(, $arg)*)
            });
        }
    };
}

bench!(create: test_create());
bench!(oneshot: test_oneshot());
bench!(inout: test_inout());

//bench!(hydra_32t_1m: test_hydra(32, 1));
//bench!(hydra_32t_1000m: test_hydra(32, 1000));
//bench!(hydra_256t_1m: test_hydra(256, 1));
//bench!(hydra_1t_1000m: test_hydra(1, 1000));
//bench!(hydra_4t_1000m: test_hydra(4, 10000));

//bench!(robin_u_32t_1m: test_robin_u(32, 1));
//bench!(robin_u_4t_1000m: test_robin_u(4, 1000));
//bench!(robin_b_32t_16m: test_robin_b(32, 16));
//bench!(robin_b_4t_1000m: test_robin_b(4, 1000));

#[derive(Clone, Copy, Debug)]
struct Input {
    threads: usize,
    messages: usize,
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}t {}m", self.threads, self.messages)
    }
}

fn hydra(c: &mut Criterion) {
    let mut g = c.benchmark_group("hydra");

    let inputs = [(32, 1), (32, 1000), (256, 1), (1, 1000), (4, 10000)];

    for (t, m) in inputs.into_iter() {
        let input = Input {
            threads: t,
            messages: m,
        };

        let id = BenchmarkId::new("flume", input);
        g.bench_with_input(id, &input, |b, i| {
            test_hydra::<flume::Sender<u32>>(b, i.threads, i.messages)
        });

        let id = BenchmarkId::new("monroe", input);
        g.bench_with_input(id, &input, |b, i| {
            test_hydra::<monroe_inbox::Sender<u32>>(b, i.threads, i.messages)
        });
    }
}

fn robin_u(c: &mut Criterion) {
    let mut g = c.benchmark_group("robin-u");

    let inputs = [(32, 1), (4, 1000)];

    for (t, m) in inputs.into_iter() {
        let input = Input {
            threads: t,
            messages: m,
        };

        let id = BenchmarkId::new("flume", input);
        g.bench_with_input(id, &input, |b, i| {
            test_robin_u::<flume::Sender<u32>>(b, i.threads, i.messages)
        });

        let id = BenchmarkId::new("monroe", input);
        g.bench_with_input(id, &input, |b, i| {
            test_robin_u::<monroe_inbox::Sender<u32>>(b, i.threads, i.messages)
        });
    }
}

fn robin_b(c: &mut Criterion) {
    let mut g = c.benchmark_group("robin-b");

    let inputs = [(32, 16), (4, 1000)];

    for (t, m) in inputs.into_iter() {
        let input = Input {
            threads: t,
            messages: m,
        };

        let id = BenchmarkId::new("flume", input);
        g.bench_with_input(id, &input, |b, i| {
            test_robin_b::<flume::Sender<u32>>(b, i.threads, i.messages)
        });

        let id = BenchmarkId::new("monroe", input);
        g.bench_with_input(id, &input, |b, i| {
            test_robin_b::<monroe_inbox::Sender<u32>>(b, i.threads, i.messages)
        });
    }
}

criterion_group!(compare, create, oneshot, inout, hydra, robin_b, robin_u,);
criterion_main!(compare);
