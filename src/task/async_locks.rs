use core::task::Poll;

use spin::{
    Mutex as SpinMutex, MutexGuard, RwLock as SpinRwLock, RwLockReadGuard as SpinRwLockReadGuard,
    RwLockWriteGuard as SpinRwLockWriteGuard,
};
pub struct MutexFuture<'a, T> {
    mutex: &'a SpinMutex<T>,
}
impl<'a, T> Future for MutexFuture<'a, T> {
    type Output = MutexGuard<'a, T>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context) -> Poll<Self::Output> {
        if let Some(guard) = self.mutex.try_lock() {
            Poll::Ready(guard)
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

pub struct ReadLockFuture<'a, T> {
    rwlock: &'a SpinRwLock<T>,
}
impl<'a, T> Future for ReadLockFuture<'a, T> {
    type Output = SpinRwLockReadGuard<'a, T>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context) -> Poll<Self::Output> {
        if let Some(guard) = self.rwlock.try_read() {
            Poll::Ready(guard)
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
pub struct WriteLockFuture<'a, T> {
    rwlock: &'a SpinRwLock<T>,
}

impl<'a, T> Future for WriteLockFuture<'a, T> {
    type Output = SpinRwLockWriteGuard<'a, T>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context) -> Poll<Self::Output> {
        if let Some(guard) = self.rwlock.try_write() {
            Poll::Ready(guard)
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
pub struct Mutex<T> {
    inner: SpinMutex<T>,
}
impl<T> Mutex<T> {
    pub fn new(data: T) -> Self {
        Mutex {
            inner: SpinMutex::new(data),
        }
    }
    pub fn lock(&self) -> MutexFuture<'_, T> {
        MutexFuture { mutex: &self.inner }
    }
    pub fn sync_lock(self) -> SpinMutex<T> {
        self.inner
    }
}
pub struct RwLock<T> {
    inner: SpinRwLock<T>,
}
impl<T> RwLock<T> {
    pub fn new(data: T) -> Self {
        RwLock {
            inner: SpinRwLock::new(data),
        }
    }
    pub fn read(&self) -> ReadLockFuture<'_, T> {
        ReadLockFuture {
            rwlock: &self.inner,
        }
    }
    pub fn write(&self) -> WriteLockFuture<'_, T> {
        WriteLockFuture {
            rwlock: &self.inner,
        }
    }
    pub fn sync_lock(self) -> SpinRwLock<T> {
        self.inner
    }
}
