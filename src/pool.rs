//! A fixed set of interchangeable items handed out one check at a time: the
//! sessions of a model, since an ONNX Runtime session runs one call at once.

use std::ops::{Deref, DerefMut};
use std::sync::{Condvar, Mutex, PoisonError};

pub struct Pool<T> {
    free: Mutex<Vec<T>>,
    returned: Condvar,
}

/// An item taken from a pool. It goes back when dropped, even while a panic
/// unwinds, so a crashed check never takes a session away for good.
pub struct Lease<'a, T> {
    pool: &'a Pool<T>,
    item: Option<T>,
}

impl<T> Pool<T> {
    /// A pool of `items`, at least one.
    pub fn new(items: Vec<T>) -> Pool<T> {
        assert!(!items.is_empty(), "a pool holds at least one item");
        Pool {
            free: Mutex::new(items),
            returned: Condvar::new(),
        }
    }

    /// A free item, waiting for one when every item is in use.
    pub fn get(&self) -> Lease<'_, T> {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(item) = free.pop() {
                return self.lease(item);
            }
            free = self
                .returned
                .wait(free)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// A free item, or `None` when every item is in use.
    pub fn try_get(&self) -> Option<Lease<'_, T>> {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        free.pop().map(|item| self.lease(item))
    }

    fn lease(&self, item: T) -> Lease<'_, T> {
        Lease {
            pool: self,
            item: Some(item),
        }
    }
}

impl<T> Drop for Lease<'_, T> {
    fn drop(&mut self) {
        if let Some(item) = self.item.take() {
            self.pool
                .free
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(item);
            self.pool.returned.notify_one();
        }
    }
}

impl<T> Deref for Lease<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.item.as_ref().expect("held until dropped")
    }
}

impl<T> DerefMut for Lease<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.item.as_mut().expect("held until dropped")
    }
}
