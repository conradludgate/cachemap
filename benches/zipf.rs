use std::cell::RefCell;
use std::hash::BuildHasher;

use cachemap3::CacheMap;
use dashmap::DashMap;
use divan::black_box;
use divan::Bencher;
use hashbrown::hash_map::DefaultHashBuilder;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use rand_distr::{DistIter, Zipf};

const N: u32 = 10000;

fn main() {
    divan::Divan::from_args()
        .threads([64])
        .sample_size(N * 10)
        .sample_count(500)
        .run_benches();
}

#[divan::bench]
fn cachemap(bencher: Bencher) {
    thread_local! {
        static RNG: RefCell<Iter> = RefCell::new(thread_rng());
    }

    let cachemap = CacheMap::with_hasher(DefaultHashBuilder::default());

    bencher
        .with_inputs(|| {
            RNG.with(|rng| {
                let mut b = rng.borrow_mut();
                (b.next().unwrap().to_bits(), b.next().unwrap().to_bits())
            })
        })
        .bench_values(|(k, v)| *cachemap.get_or_insert(&k, || v));
}

#[divan::bench]
fn dashmap(bencher: Bencher) {
    thread_local! {
        static RNG: RefCell<Iter> = RefCell::new(thread_rng());
    }

    let dashmap = DashMap::with_hasher(DefaultHashBuilder::default());

    bencher
        .with_inputs(|| {
            RNG.with(|rng| {
                let mut b = rng.borrow_mut();
                (b.next().unwrap().to_bits(), b.next().unwrap().to_bits())
            })
        })
        .bench_values(|(k, v)| *dashmap.entry(k).or_insert_with(|| v).downgrade());
}

#[divan::bench]
fn dashmap_read(bencher: Bencher) {
    thread_local! {
        static RNG: RefCell<Iter> = RefCell::new(thread_rng());
    }

    let dashmap = DashMap::with_hasher(DefaultHashBuilder::default());

    bencher
        .with_inputs(|| {
            RNG.with(|rng| {
                let mut b = rng.borrow_mut();
                (b.next().unwrap().to_bits(), b.next().unwrap().to_bits())
            })
        })
        .bench_values(|(k, v)| 'foo: {
            if let Some(k) = dashmap.get(&k) {
                break 'foo *k;
            }
            *dashmap.entry(k).or_insert_with(|| v).downgrade()
        });
}

type Iter = DistIter<Zipf<f64>, SmallRng, f64>;
fn thread_rng() -> Iter {
    SmallRng::seed_from_u64(DefaultHashBuilder::default().hash_one(std::thread::current().id()))
        .sample_iter(Zipf::new(N as u64, 1.5).unwrap())
}
