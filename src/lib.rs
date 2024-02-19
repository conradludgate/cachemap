#![cfg_attr(not(feature = "std"), no_std)]

mod lock;
pub mod raw;

pub type MappedRwLockReadGuard<'a, T> = lock_api::MappedRwLockReadGuard<'a, lock::RawRwLock, T>;

#[cfg(feature = "std")]
pub type CacheMapShard<K, V, S = hashbrown::hash_map::DefaultHashBuilder> =
    raw::RawCacheMapShard<K, V, S, lock::RawRwLock, parking_lot::RawMutex>;

#[cfg(feature = "std")]
pub struct CacheMap<K, V, S = hashbrown::hash_map::DefaultHashBuilder> {
    shift: u32,
    shards: Box<[raw::RawCacheMapShardInner<K, V, lock::RawRwLock, parking_lot::RawMutex>]>,
    hasher: S,
}

#[cfg(feature = "std")]
// taken from dashmap
fn default_shard_amount() -> usize {
    use std::sync::OnceLock;

    static DEFAULT_SHARD_AMOUNT: OnceLock<usize> = OnceLock::new();
    *DEFAULT_SHARD_AMOUNT.get_or_init(|| {
        (std::thread::available_parallelism().map_or(1, usize::from) * 4).next_power_of_two()
    })
}

#[cfg(feature = "std")]
impl<K, V, S> CacheMap<K, V, S>
where
    K: core::hash::Hash + Eq + Clone,
    V: Clone,
    S: core::hash::BuildHasher,
{
    pub fn with_hasher(hasher: S) -> Self {
        let shards = default_shard_amount();
        let mut vec = Vec::with_capacity(shards);
        vec.resize_with(shards, raw::RawCacheMapShardInner::default);
        Self {
            shift: (std::mem::size_of::<usize>() * 8) as u32 - shards.trailing_zeros(),
            shards: vec.into_boxed_slice(),
            hasher,
        }
    }

    pub fn get_or_insert(
        &self,
        key: &K,
        value: impl FnOnce() -> V,
    ) -> MappedRwLockReadGuard<'_, (K, V)> {
        let hash = self.hasher.hash_one(key);
        let shard = unsafe {
            self.shards
                .get_unchecked(((hash as usize) << 7) >> self.shift)
        };
        shard.get_or_insert(hash, key, value, &self.hasher)
    }
}
