use core::hash::{BuildHasher, Hash};

use hashbrown::raw::RawTable;
use lock_api::{
    MappedRwLockReadGuard, RawRwLockUpgradeDowngrade, RwLock, RwLockReadGuard,
    RwLockUpgradableReadGuard, RwLockWriteGuard,
};

pub struct RawCacheMapShard<K, V, S, Lock> {
    inner: RawCacheMapShardInner<K, V, Lock>,
    hasher: S,
}

impl<K, V, S, Lock> RawCacheMapShard<K, V, S, Lock>
where
    K: Hash + Eq + Clone,
    V: Clone,
    S: BuildHasher,
    Lock: RawRwLockUpgradeDowngrade,
{
    pub const fn new(hasher: S) -> Self {
        Self {
            inner: RawCacheMapShardInner::new(),
            hasher,
        }
    }

    pub fn get_or_insert(
        &self,
        key: &K,
        value: impl FnOnce() -> V,
    ) -> MappedRwLockReadGuard<'_, Lock, (K, V)> {
        let hash = self.hasher.hash_one(key);
        self.inner.get_or_insert(hash, key, value, &self.hasher)
    }
}

pub struct RawCacheMapShardInner<K, V, Lock> {
    table: RwLock<Lock, RawTable<(K, V)>>,
}

impl<K, V, Lock> RawCacheMapShardInner<K, V, Lock>
where
    K: Hash + Eq + Clone,
    V: Clone,
    Lock: RawRwLockUpgradeDowngrade,
{
    pub const fn new() -> Self {
        Self {
            table: RwLock::new(RawTable::new()),
        }
    }

    pub fn get_or_insert(
        &self,
        hash: u64,
        key: &K,
        value: impl FnOnce() -> V,
        hasher: &impl BuildHasher,
    ) -> MappedRwLockReadGuard<'_, Lock, (K, V)> {
        // fast path
        if let Ok(mapped) =
            RwLockReadGuard::try_map(self.table.read(), |map| map.get(hash, |k| k.0 == *key))
        {
            return mapped;
        }

        // acquire a unique lock for the insert routine.
        //
        // > The calling thread will be blocked until there are no more writers or other upgradable reads which hold the lock.
        // > There may be other readers currently inside the lock when this method returns.
        let upgradeable = self.table.upgradable_read();

        // potential race condition. double check now we re-locked
        if let Some(bucket) = upgradeable.find(hash, |k| k.0 == *key) {
            // SAFETY: no safety comment but I assume this is ok as long as the bucket is from this rawtable.
            let index = unsafe { upgradeable.bucket_index(&bucket) };
            let read = RwLockUpgradableReadGuard::downgrade(upgradeable);
            // SAFETY:
            // 1. the table is allocated (we have an entry)
            // 2. the index is definitely in bounds of the buckets
            // 3. the bucket is occupied
            return RwLockReadGuard::map(read, |map| unsafe { map.bucket(index).as_ref() });
        }

        let mut write = if upgradeable.len() == upgradeable.capacity() {
            let mut realloc = RawTable::with_capacity(upgradeable.len() + 1);
            realloc.clone_from_with_hasher(&*upgradeable, |k| hasher.hash_one(&k.0));
            let mut write = RwLockUpgradableReadGuard::upgrade(upgradeable);
            *write = realloc;
            write
        } else {
            RwLockUpgradableReadGuard::upgrade(upgradeable)
        };

        // SAFETY: we have allocated enough space above.
        let bucket = unsafe { write.insert_no_grow(hash, (key.clone(), value())) };

        // SAFETY: no safety comment but I assume this is ok as long as the bucket is from this rawtable.
        let index = unsafe { write.bucket_index(&bucket) };
        let read = RwLockWriteGuard::downgrade(write);
        // SAFETY:
        // 1. the table is allocated (we have just inserted an entry)
        // 2. the index is definitely in bounds of the buckets
        // 3. the bucket is occupied
        return RwLockReadGuard::map(read, |map| unsafe { map.bucket(index).as_ref() });
    }
}

impl<K, V, Lock> Default for RawCacheMapShardInner<K, V, Lock>
where
    K: Hash + Eq + Clone,
    V: Clone,
    Lock: RawRwLockUpgradeDowngrade,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use hashbrown::hash_map::DefaultHashBuilder;
    use parking_lot::RawRwLock;

    use super::RawCacheMapShard;

    #[test]
    fn test() {
        let cache = RawCacheMapShard::<usize, usize, DefaultHashBuilder, RawRwLock>::new(
            DefaultHashBuilder::default(),
        );

        let cache_0 = cache.get_or_insert(&0, || 0);
        let cache_1 = cache.get_or_insert(&0, || 1);

        assert_eq!(*cache_0, (0, 0));
        assert_eq!(*cache_1, (0, 0));
    }
}
