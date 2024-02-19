use core::hash::{BuildHasher, Hash};

use hashbrown::raw::RawTable;
use lock_api::{
    MappedRwLockReadGuard, Mutex, RawMutex, RawRwLock, RawRwLockDowngrade, RwLock, RwLockReadGuard,
    RwLockWriteGuard,
};

pub struct RawCacheMapShard<K, V, S, Rw, Lock> {
    inner: RawCacheMapShardInner<K, V, Rw, Lock>,
    hasher: S,
}

impl<K, V, S, Rw, Lock> RawCacheMapShard<K, V, S, Rw, Lock>
where
    K: Hash + Eq + Clone,
    V: Clone,
    S: BuildHasher,
    Rw: RawRwLockDowngrade,
    Lock: RawMutex,
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
    ) -> MappedRwLockReadGuard<'_, Rw, (K, V)> {
        let hash = self.hasher.hash_one(key);
        self.inner.get_or_insert(hash, key, value, &self.hasher)
    }
}

pub struct RawCacheMapShardInner<K, V, Rw, Lock> {
    table: RwLock<Rw, RawTable<(K, V)>>,
    insert: Mutex<Lock, ()>,
}

impl<K, V, Rw, Lock> RawCacheMapShardInner<K, V, Rw, Lock>
where
    K: Hash + Eq + Clone,
    V: Clone,
    Rw: RawRwLockDowngrade,
    Lock: RawMutex,
{
    pub const fn new() -> Self {
        Self {
            table: RwLock::new(RawTable::new()),
            insert: Mutex::new(()),
        }
    }

    pub fn get(&self, hash: u64, key: &K) -> Option<MappedRwLockReadGuard<'_, Rw, (K, V)>> {
        RwLockReadGuard::try_map(self.table.read(), |map| map.get(hash, |k| k.0 == *key)).ok()
    }

    pub fn get_or_insert(
        &self,
        hash: u64,
        key: &K,
        value: impl FnOnce() -> V,
        hasher: &impl BuildHasher,
    ) -> MappedRwLockReadGuard<'_, Rw, (K, V)> {
        // fast path
        let mut read =
            match RwLockReadGuard::try_map(self.table.read(), |map| map.get(hash, |k| k.0 == *key))
            {
                Ok(mapped) => return mapped,
                Err(read) => read,
            };

        // acquire a unique lock for the insert routine.
        let _guard = RwLockReadGuard::unlocked(&mut read, || self.insert.lock());

        // potential race condition. double check now we re-locked
        let mut read = match RwLockReadGuard::try_map(read, |map| map.get(hash, |k| k.0 == *key)) {
            Ok(mapped) => return mapped,
            Err(read) => read,
        };

        let index = if read.len() == read.capacity() {
            let mut realloc = RawTable::with_capacity(read.len() + 1);
            realloc.clone_from_with_hasher(&*read, |k| hasher.hash_one(&k.0));

            // SAFETY: we have allocated enough space above.
            let bucket = unsafe { realloc.insert_no_grow(hash, (key.clone(), value())) };
            // SAFETY: no safety comment but I assume this is ok as long as the bucket is from this rawtable.
            let index = unsafe { realloc.bucket_index(&bucket) };

            drop(read);
            let mut write = self.table.write();
            *write = realloc;
            read = RwLockWriteGuard::downgrade(write);

            index
        } else {
            drop(read);

            let mut write = self.table.write();
            // SAFETY: we have allocated enough space above.
            let bucket = unsafe { write.insert_no_grow(hash, (key.clone(), value())) };
            // SAFETY: no safety comment but I assume this is ok as long as the bucket is from this rawtable.
            let index = unsafe { write.bucket_index(&bucket) };

            read = RwLockWriteGuard::downgrade(write);

            index
        };

        drop(_guard);

        // SAFETY:
        // 1. the table is allocated (we have just inserted an entry)
        // 2. the index is definitely in bounds of the buckets
        // 3. the bucket is occupied
        return RwLockReadGuard::map(read, |map| unsafe { map.bucket(index).as_ref() });
    }
}

impl<K, V, Rw, Lock> Default for RawCacheMapShardInner<K, V, Rw, Lock>
where
    K: Hash + Eq + Clone,
    V: Clone,
    Rw: RawRwLockDowngrade,
    Lock: RawMutex,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use hashbrown::hash_map::DefaultHashBuilder;
    use parking_lot::{RawMutex, RawRwLock};

    use super::RawCacheMapShard;

    #[test]
    fn test() {
        let cache = RawCacheMapShard::<usize, usize, DefaultHashBuilder, RawRwLock, RawMutex>::new(
            DefaultHashBuilder::default(),
        );

        let cache_0 = cache.get_or_insert(&0, || 0);
        let cache_1 = cache.get_or_insert(&0, || 1);

        assert_eq!(*cache_0, (0, 0));
        assert_eq!(*cache_1, (0, 0));
    }
}
