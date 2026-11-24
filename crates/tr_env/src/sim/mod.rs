//! single-threaded deterministic world: executor, virtual time, per-node
//! clocks with skew, an unreliable network, node crash and restart.
//! everything iterated here is a Vec, VecDeque or BTreeMap, and every tie is
//! broken by a sequence number, so a seed replays bit for bit.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::future::Future;
use std::hash::Hasher;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;
use std::time::Duration;

use xxhash_rust::xxh3::Xxh3;

use crate::Addr;
use crate::BoxFuture;
use crate::Clock;
use crate::Env;
use crate::Instant;
use crate::NodeId;
use crate::Rng;
use crate::Runtime;
use crate::Spawn;
use crate::Transport;
use crate::WallTime;

/// 2026-01-01t00:00:00z, so wall clocks look like real timestamps.
const EPOCH_NS: u64 = 1_767_225_600_000_000_000;
const MAX_POLLS_PER_INSTANT: u64 = 1_000_000;

#[derive(Clone, Debug)]
pub struct NetConfig {
    /// per mille
    pub loss: u64,
    /// per mille
    pub duplicate: u64,
    pub min_latency: Duration,
    pub max_latency: Duration,
}

#[derive(Clone, Debug)]
pub struct SimConfig {
    pub seed: u64,
    pub net: NetConfig,
    pub max_clock_skew: Duration,
}

pub type Boot = Arc<dyn Fn(Env) -> BoxFuture<'static, ()> + Send + Sync>;

struct Task {
    node: NodeId,
    fut: BoxFuture<'static, ()>,
}

enum Event {
    Wake(Waker),
    Deliver(NodeId, Addr, Vec<u8>),
}

struct NodeSlot {
    addr: Addr,
    up: bool,
    incarnation: u64,
    inbox: VecDeque<(Addr, Vec<u8>)>,
    recv_waker: Option<Waker>,
    wall_offset_ns: i64,
    rng: Rng,
    boot: Boot,
}

struct State {
    now: u64,
    seq: u64,
    next_task: u64,
    events: BTreeMap<(u64, u64), Event>,
    spawned: Vec<(u64, Task)>,
    nodes: Vec<NodeSlot>,
    net_rng: Rng,
    blocked: BTreeSet<(NodeId, NodeId)>,
    cfg: SimConfig,
    trace: Xxh3,
}

struct Shared {
    st: Mutex<State>,
    /// separate lock so a waker can fire while st is held.
    ready: Mutex<Vec<u64>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct TaskWaker {
    id: u64,
    shared: Arc<Shared>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        lock(&self.shared.ready).push(self.id);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        lock(&self.shared.ready).push(self.id);
    }
}

pub struct World {
    shared: Arc<Shared>,
    tasks: BTreeMap<u64, Task>,
    sched_rng: Rng,
    polls_this_instant: u64,
    last_instant: u64,
}

impl World {
    pub fn new(cfg: SimConfig) -> World {
        // independent streams so that, for example, a new network fault
        // knob does not shift the scheduler.
        let mut root = Rng::new(cfg.seed);
        let sched_rng = root.fork();
        let net_rng = root.fork();
        let st = State {
            now: 0,
            seq: 0,
            next_task: 0,
            events: BTreeMap::new(),
            spawned: Vec::new(),
            nodes: Vec::new(),
            net_rng,
            blocked: BTreeSet::new(),
            cfg,
            trace: Xxh3::with_seed(0),
        };
        World {
            shared: Arc::new(Shared {
                st: Mutex::new(st),
                ready: Mutex::new(Vec::new()),
            }),
            tasks: BTreeMap::new(),
            sched_rng,
            polls_this_instant: 0,
            last_instant: 0,
        }
    }

    pub fn now(&self) -> Instant {
        Instant(lock(&self.shared.st).now)
    }

    pub fn add_node(&mut self, boot: Boot) -> NodeId {
        let id = {
            let mut st = lock(&self.shared.st);
            let id = NodeId(st.nodes.len() as u64);
            let skew = st.cfg.max_clock_skew.as_nanos() as u64;
            let off = st.net_rng.below(2 * skew + 1) as i64 - skew as i64;
            let seed = st.cfg.seed ^ (id.0 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let addr = Addr(format!("n{}", id.0));
            st.nodes.push(NodeSlot {
                addr,
                up: false,
                incarnation: 0,
                inbox: VecDeque::new(),
                recv_waker: None,
                wall_offset_ns: off,
                rng: Rng::new(seed),
                boot,
            });
            id
        };
        self.start(id);
        id
    }

    pub fn addr(&self, n: NodeId) -> Addr {
        lock(&self.shared.st).nodes[n.0 as usize].addr.clone()
    }

    fn start(&mut self, n: NodeId) {
        let boot = {
            let mut st = lock(&self.shared.st);
            let slot = &mut st.nodes[n.0 as usize];
            slot.up = true;
            slot.boot.clone()
        };
        let rt = Arc::new(NodeRt {
            node: n,
            shared: self.shared.clone(),
        });
        let env = Env::new(rt.clone());
        rt.spawn_boxed("boot", boot(env));
    }

    /// drops every task of the node and its inbox. disk state (not in this
    /// prototype) would keep only what was synced.
    pub fn crash(&mut self, n: NodeId) {
        let ids: Vec<u64> = self
            .tasks
            .iter()
            .filter(|(_, t)| t.node == n)
            .map(|(id, _)| *id)
            .collect();
        let mut dead: Vec<Task> = ids.iter().filter_map(|id| self.tasks.remove(id)).collect();
        let waker = {
            let mut st = lock(&self.shared.st);
            let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut st.spawned)
                .into_iter()
                .partition(|(_, t)| t.node != n);
            st.spawned = keep;
            dead.extend(gone.into_iter().map(|(_, t)| t));
            st.trace.write_u64(0xc4a5);
            st.trace.write_u64(n.0);
            let slot = &mut st.nodes[n.0 as usize];
            slot.up = false;
            slot.incarnation += 1;
            slot.inbox.clear();
            slot.recv_waker.take()
        };
        // destructors may touch the world, so drop outside the lock.
        drop(waker);
        drop(dead);
    }

    pub fn restart(&mut self, n: NodeId) {
        self.start(n);
    }

    pub fn is_up(&self, n: NodeId) -> bool {
        lock(&self.shared.st).nodes[n.0 as usize].up
    }

    /// drop everything sent from a to b.
    pub fn block(&mut self, a: NodeId, b: NodeId) {
        lock(&self.shared.st).blocked.insert((a, b));
    }

    pub fn heal(&mut self) {
        lock(&self.shared.st).blocked.clear();
    }

    pub fn set_net(&mut self, net: NetConfig) {
        lock(&self.shared.st).cfg.net = net;
    }

    pub fn trace_hash(&self) -> u64 {
        lock(&self.shared.st).trace.clone().finish()
    }

    /// runs until virtual time reaches limit or nothing can happen.
    /// returns false if the world went quiet before limit.
    pub fn run_until(&mut self, limit: Instant) -> bool {
        loop {
            let spawned = std::mem::take(&mut lock(&self.shared.st).spawned);
            for (id, t) in spawned {
                self.tasks.insert(id, t);
            }
            let next = {
                let mut ready = lock(&self.shared.ready);
                if ready.is_empty() {
                    None
                } else {
                    let i = self.sched_rng.below(ready.len() as u64) as usize;
                    Some(ready.swap_remove(i))
                }
            };
            if let Some(id) = next {
                self.poll_task(id);
                continue;
            }
            if !self.fire_next_event(limit) {
                return lock(&self.shared.st).now >= limit.0;
            }
        }
    }

    fn poll_task(&mut self, id: u64) {
        let Some(task) = self.tasks.get_mut(&id) else {
            // woken after it finished or after its node crashed.
            return;
        };
        let now = {
            let mut st = lock(&self.shared.st);
            let now = st.now;
            st.trace.write_u64(now);
            st.trace.write_u64(id);
            now
        };
        if now != self.last_instant {
            self.last_instant = now;
            self.polls_this_instant = 0;
        }
        self.polls_this_instant += 1;
        assert!(
            self.polls_this_instant < MAX_POLLS_PER_INSTANT,
            "livelock: virtual time stopped advancing"
        );
        let waker = Waker::from(Arc::new(TaskWaker {
            id,
            shared: self.shared.clone(),
        }));
        let mut cx = Context::from_waker(&waker);
        if task.fut.as_mut().poll(&mut cx).is_ready() {
            self.tasks.remove(&id);
        }
    }

    fn fire_next_event(&mut self, limit: Instant) -> bool {
        let mut st = lock(&self.shared.st);
        let Some((&(t, seq), _)) = st.events.iter().next() else {
            return false;
        };
        if t > limit.0 {
            st.now = limit.0;
            return false;
        }
        let ev = st.events.remove(&(t, seq));
        st.now = t;
        match ev {
            Some(Event::Wake(w)) => w.wake(),
            Some(Event::Deliver(to, from, msg)) => {
                st.trace.write_u64(t);
                st.trace.write_u64(to.0);
                st.trace.write(&msg);
                let slot = &mut st.nodes[to.0 as usize];
                if slot.up {
                    slot.inbox.push_back((from, msg));
                    if let Some(w) = slot.recv_waker.take() {
                        w.wake();
                    }
                }
            }
            None => {}
        }
        true
    }
}

struct NodeRt {
    node: NodeId,
    shared: Arc<Shared>,
}

impl Runtime for NodeRt {
    fn node(&self) -> NodeId {
        self.node
    }
    fn clock(&self) -> &dyn Clock {
        self
    }
    fn spawner(&self) -> &dyn Spawn {
        self
    }
    fn transport(&self) -> &dyn Transport {
        self
    }
    fn root_random(&self) -> u64 {
        lock(&self.shared.st).nodes[self.node.0 as usize]
            .rng
            .next_u64()
    }
}

impl Clock for NodeRt {
    fn now(&self) -> Instant {
        Instant(lock(&self.shared.st).now)
    }
    fn wall(&self) -> WallTime {
        let st = lock(&self.shared.st);
        let off = st.nodes[self.node.0 as usize].wall_offset_ns;
        WallTime((EPOCH_NS + st.now).saturating_add_signed(off))
    }
    fn sleep_until(&self, deadline: Instant) -> BoxFuture<'static, ()> {
        Box::pin(Sleep {
            shared: self.shared.clone(),
            deadline: deadline.0,
            key: None,
        })
    }
}

struct Sleep {
    shared: Arc<Shared>,
    deadline: u64,
    key: Option<(u64, u64)>,
}

impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let shared = self.shared.clone();
        let mut st = lock(&shared.st);
        if st.now >= self.deadline {
            if let Some(k) = self.key.take() {
                st.events.remove(&k);
            }
            return Poll::Ready(());
        }
        let key = match self.key {
            Some(k) => k,
            None => {
                st.seq += 1;
                (self.deadline, st.seq)
            }
        };
        st.events.insert(key, Event::Wake(cx.waker().clone()));
        self.key = Some(key);
        Poll::Pending
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some(k) = self.key.take() {
            let ev = lock(&self.shared.st).events.remove(&k);
            drop(ev);
        }
    }
}

impl Spawn for NodeRt {
    fn spawn_boxed(&self, _name: &'static str, fut: BoxFuture<'static, ()>) {
        let mut st = lock(&self.shared.st);
        if !st.nodes[self.node.0 as usize].up {
            drop(st);
            drop(fut);
            return;
        }
        let id = st.next_task;
        st.next_task += 1;
        let task = Task {
            node: self.node,
            fut,
        };
        st.spawned.push((id, task));
        drop(st);
        lock(&self.shared.ready).push(id);
    }
}

impl Transport for NodeRt {
    fn local_addr(&self) -> Addr {
        lock(&self.shared.st).nodes[self.node.0 as usize]
            .addr
            .clone()
    }

    fn send(&self, to: &Addr, msg: Vec<u8>) -> io::Result<()> {
        let mut st = lock(&self.shared.st);
        let from_addr = st.nodes[self.node.0 as usize].addr.clone();
        let Some(dst) = st.nodes.iter().position(|n| &n.addr == to) else {
            return Ok(());
        };
        let dst = NodeId(dst as u64);
        if st.blocked.contains(&(self.node, dst)) {
            return Ok(());
        }
        let net = st.cfg.net.clone();
        if st.net_rng.chance(net.loss, 1000) {
            return Ok(());
        }
        let mut copies = 1;
        if st.net_rng.chance(net.duplicate, 1000) {
            copies = 2;
        }
        let lo = net.min_latency.as_nanos() as u64;
        let hi = net.max_latency.as_nanos() as u64 + 1;
        for _ in 0..copies {
            let lat = st.net_rng.range(lo, hi);
            st.seq += 1;
            let key = (st.now + lat, st.seq);
            let ev = Event::Deliver(dst, from_addr.clone(), msg.clone());
            st.events.insert(key, ev);
        }
        Ok(())
    }

    fn recv(&self) -> BoxFuture<'_, (Addr, Vec<u8>)> {
        Box::pin(std::future::poll_fn(move |cx| {
            let mut st = lock(&self.shared.st);
            let slot = &mut st.nodes[self.node.0 as usize];
            if let Some(m) = slot.inbox.pop_front() {
                return Poll::Ready(m);
            }
            slot.recv_waker = Some(cx.waker().clone());
            Poll::Pending
        }))
    }
}
