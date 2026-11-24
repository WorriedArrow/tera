//! research/12 prototype of the tr_env surface. never merges.

pub mod collections;
pub mod rng;
pub mod sim;
pub mod sync;

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

pub use rng::Rng;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// node identity inside one process. the sim runs many nodes on one thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

/// monotonic time since the node's runtime started. never goes backwards.
/// not std::time::Instant because the sim must be able to construct it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(pub u64);

impl Instant {
    pub fn after(self, d: Duration) -> Instant {
        Instant(self.0.saturating_add(d.as_nanos() as u64))
    }
    pub fn since(self, earlier: Instant) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }
}

/// wall clock, nanoseconds since the unix epoch. may jump and may be skewed
/// against other nodes. hlc (tr_mvcc) is the only consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallTime(pub u64);

/// node-to-node address. stable across restarts of the same node.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Addr(pub String);

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn wall(&self) -> WallTime;
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'static, ()>;
}

pub trait Spawn: Send + Sync {
    fn spawn_boxed(&self, name: &'static str, fut: BoxFuture<'static, ()>);
}

/// unreliable datagrams between nodes: may be lost, delayed, reordered or
/// duplicated. raft assumes exactly this (raft paper section 5.1).
pub trait Transport: Send + Sync {
    fn local_addr(&self) -> Addr;
    /// never blocks. an error only for programming errors (oversized message).
    fn send(&self, to: &Addr, msg: Vec<u8>) -> io::Result<()>;
    /// next message for this node. one consumer per node, demux above.
    fn recv(&self) -> BoxFuture<'_, (Addr, Vec<u8>)>;
}

/// files with explicit durability. writes are volatile until sync.
/// owned buffers so the prod impl can hand them to a blocking pool (and to
/// io_uring later) without borrowing across threads.
pub trait Fs: Send + Sync {
    fn open(&self, path: &str, opts: OpenOptions) -> BoxFuture<'_, io::Result<Box<dyn File>>>;
    fn remove(&self, path: &str) -> BoxFuture<'_, io::Result<()>>;
    fn rename(&self, from: &str, to: &str) -> BoxFuture<'_, io::Result<()>>;
    fn create_dir(&self, path: &str) -> BoxFuture<'_, io::Result<()>>;
    /// sorted names, so prod and sim iterate the same way.
    fn list(&self, dir: &str) -> BoxFuture<'_, io::Result<Vec<String>>>;
    /// makes creates, renames and removes in dir durable.
    fn sync_dir(&self, dir: &str) -> BoxFuture<'_, io::Result<()>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OpenOptions {
    pub create: bool,
    pub create_new: bool,
    pub truncate: bool,
}

pub trait File: Send + Sync {
    fn read_at(&self, offset: u64, len: usize) -> BoxFuture<'_, io::Result<Vec<u8>>>;
    fn write_at(&self, offset: u64, data: Vec<u8>) -> BoxFuture<'_, io::Result<()>>;
    /// fdatasync. an error poisons the file: every later call fails.
    fn sync(&self) -> BoxFuture<'_, io::Result<()>>;
    fn size(&self) -> BoxFuture<'_, io::Result<u64>>;
    fn set_len(&self, len: u64) -> BoxFuture<'_, io::Result<()>>;
}

/// reliable ordered byte streams for client-facing ports (pgwire, tera sql).
pub trait Streams: Send + Sync {
    fn listen(&self, port: u16) -> BoxFuture<'_, io::Result<Box<dyn Listener>>>;
    fn connect(&self, addr: &Addr) -> BoxFuture<'_, io::Result<Box<dyn Stream>>>;
}

pub trait Listener: Send {
    fn accept(&mut self) -> BoxFuture<'_, io::Result<(Box<dyn Stream>, Addr)>>;
}

pub trait Stream: Send {
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, io::Result<usize>>;
    fn write<'a>(&'a mut self, buf: &'a [u8]) -> BoxFuture<'a, io::Result<usize>>;
    fn shutdown(&mut self) -> BoxFuture<'_, io::Result<()>>;
}

/// everything a node can touch. one impl per world (prod or sim).
pub trait Runtime: Send + Sync + 'static {
    fn node(&self) -> NodeId;
    fn clock(&self) -> &dyn Clock;
    fn spawner(&self) -> &dyn Spawn;
    fn transport(&self) -> &dyn Transport;
    /// next value of the node root random stream. components fork an owned
    /// Rng from it once instead of sharing a locked one.
    fn root_random(&self) -> u64;
}

/// the handle core crates hold. cheap to clone, no generic parameter.
#[derive(Clone)]
pub struct Env {
    rt: Arc<dyn Runtime>,
}

impl Env {
    pub fn new(rt: Arc<dyn Runtime>) -> Env {
        Env { rt }
    }
    pub fn node(&self) -> NodeId {
        self.rt.node()
    }
    pub fn now(&self) -> Instant {
        self.rt.clock().now()
    }
    pub fn wall(&self) -> WallTime {
        self.rt.clock().wall()
    }
    pub fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        let deadline = self.now().after(d);
        self.rt.clock().sleep_until(deadline)
    }
    pub fn rng(&self) -> Rng {
        Rng::new(self.rt.root_random())
    }
    pub fn transport(&self) -> &dyn Transport {
        self.rt.transport()
    }
    pub fn spawn<T, F>(&self, name: &'static str, fut: F) -> JoinHandle<T>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = sync::oneshot::<T>();
        self.rt.spawner().spawn_boxed(
            name,
            Box::pin(async move {
                tx.send(fut.await);
            }),
        );
        JoinHandle { rx }
    }
    /// first of fut and the deadline. polls fut first, always, so the outcome
    /// never depends on a random branch order (tokio::select! randomizes).
    pub async fn timeout<T, F>(&self, d: Duration, fut: F) -> Option<T>
    where
        F: Future<Output = T>,
    {
        let mut sleep = self.sleep(d);
        let mut fut = std::pin::pin!(fut);
        std::future::poll_fn(move |cx: &mut Context<'_>| {
            if let Poll::Ready(v) = fut.as_mut().poll(cx) {
                return Poll::Ready(Some(v));
            }
            if sleep.as_mut().poll(cx).is_ready() {
                return Poll::Ready(None);
            }
            Poll::Pending
        })
        .await
    }
}

/// resolves to the task output, or None if the task was dropped (its node
/// crashed). dropping the handle detaches the task.
pub struct JoinHandle<T> {
    rx: sync::OneshotReceiver<T>,
}

impl<T> Future for JoinHandle<T> {
    type Output = Option<T>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        Pin::new(&mut self.rx).poll(cx)
    }
}
