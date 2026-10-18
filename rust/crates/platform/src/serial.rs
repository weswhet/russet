//! Locks that keep recipes running in parallel from overlapping on a shared
//! resource, such as a disk image or the packaging helper.
//!
//! A lock is reentrant: a thread that holds a key can take it again, because
//! a processor can run another processor, or mount an image it already
//! mounted, on the same thread.
use std::{
    sync::{Condvar, Mutex},
    thread::ThreadId,
};

/// Holders of each key: the owning thread and how many times it took the key.
pub struct KeyedLock<K> {
    held: Mutex<Vec<(K, ThreadId, usize)>>,
    released: Condvar,
}

impl<K: PartialEq + Clone> KeyedLock<K> {
    pub const fn new() -> Self {
        Self {
            held: Mutex::new(Vec::new()),
            released: Condvar::new(),
        }
    }

    /// Blocks until no other thread holds `key`, then holds it until the
    /// guard drops.
    pub fn lock(&'static self, key: K) -> KeyedGuard<K> {
        let me = std::thread::current().id();
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match held.iter_mut().find(|(k, _, _)| *k == key) {
                None => {
                    held.push((key.clone(), me, 1));
                    break;
                }
                Some((_, owner, count)) if *owner == me => {
                    *count += 1;
                    break;
                }
                Some(_) => {
                    held = self.released.wait(held).unwrap_or_else(|e| e.into_inner());
                }
            }
        }
        KeyedGuard { lock: self, key }
    }
}

impl<K: PartialEq + Clone> Default for KeyedLock<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// Releases one hold on a key when dropped.
pub struct KeyedGuard<K: PartialEq + Clone + 'static> {
    lock: &'static KeyedLock<K>,
    key: K,
}

impl<K: PartialEq + Clone + 'static> Drop for KeyedGuard<K> {
    fn drop(&mut self) {
        let mut held = self.lock.held.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = held.iter().position(|(k, _, _)| *k == self.key) {
            held[index].2 -= 1;
            if held[index].2 == 0 {
                held.swap_remove(index);
                self.lock.released.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[test]
    fn a_thread_can_take_its_own_key_again() {
        static LOCK: KeyedLock<u8> = KeyedLock::new();
        let outer = LOCK.lock(1);
        let inner = LOCK.lock(1);
        drop(inner);
        drop(outer);
        assert!(LOCK.held.lock().unwrap().is_empty());
    }

    #[test]
    fn threads_wait_for_the_same_key_but_not_for_others() {
        static LOCK: KeyedLock<u8> = KeyedLock::new();
        static INSIDE: AtomicUsize = AtomicUsize::new(0);
        static MOST: AtomicUsize = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let _guard = LOCK.lock(7);
                    let now = INSIDE.fetch_add(1, Ordering::SeqCst) + 1;
                    MOST.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    INSIDE.fetch_sub(1, Ordering::SeqCst);
                });
            }
            // A different key never waits on key 7.
            scope.spawn(|| drop(LOCK.lock(8)));
        });
        assert_eq!(MOST.load(Ordering::SeqCst), 1);
        assert!(LOCK.held.lock().unwrap().is_empty());
    }
}
