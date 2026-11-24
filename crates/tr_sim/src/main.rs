//! research/12 prototype: two-node echo under loss, duplication, reordering,
//! partitions, clock skew and server crashes. never merges.

// the prototype cli reads args, prints and times itself. in the real layout
// this lives in xtask and tr_sim stays under the determinism list.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use tr_env::Addr;
use tr_env::Env;
use tr_env::Instant;
use tr_env::Rng;
use tr_env::sim::NetConfig;
use tr_env::sim::SimConfig;
use tr_env::sim::World;

const REQUESTS: u64 = 300;
const MS: u64 = 1_000_000;

#[derive(Default, Debug, PartialEq, Eq)]
struct Log {
    sent: BTreeMap<u64, Vec<u8>>,
    completed: u64,
    errors: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    trace: u64,
    completed: u64,
    errors: Vec<String>,
    end_ms: u64,
}

async fn echo_server(env: Env) {
    let mut rng = env.rng();
    loop {
        let (from, msg) = env.transport().recv().await;
        env.sleep(Duration::from_nanos(rng.below(5 * MS))).await;
        let _ = env.transport().send(&from, msg);
    }
}

fn payload(env: &Env, id: u64, nondeterministic: bool) -> Vec<u8> {
    let mut rng = env.rng();
    let mut p = id.to_le_bytes().to_vec();
    for _ in 0..8 {
        p.push(rng.below(256) as u8);
    }
    if nondeterministic {
        // deliberate bug: std HashSet iteration order comes from RandomState.
        let set: std::collections::HashSet<u8> = (0u8..16).collect();
        p.push(*set.iter().next().unwrap_or(&0));
    }
    p
}

async fn client(env: Env, server: Addr, log: Arc<Mutex<Log>>, nondeterministic: bool) {
    for id in 0..REQUESTS {
        let p = payload(&env, id, nondeterministic);
        log.lock().unwrap().sent.insert(id, p.clone());
        loop {
            let _ = env.transport().send(&server, p.clone());
            let got = env
                .timeout(Duration::from_millis(100), wait_reply(&env, id, &log))
                .await;
            if got.is_some() {
                break;
            }
        }
        log.lock().unwrap().completed += 1;
    }
}

async fn wait_reply(env: &Env, id: u64, log: &Mutex<Log>) {
    loop {
        let (_, msg) = env.transport().recv().await;
        let mut l = log.lock().unwrap();
        let rid = u64::from_le_bytes(msg[..8].try_into().unwrap());
        let verdict = match l.sent.get(&rid) {
            Some(p) if *p == msg => None,
            Some(_) => Some("mismatch"),
            None => Some("never requested"),
        };
        if let Some(v) = verdict {
            l.errors.push(format!("reply {rid}: {v}"));
        }
        if rid == id {
            return;
        }
    }
}

fn run(seed: u64, nondeterministic: bool) -> Outcome {
    let net = NetConfig {
        loss: 100,
        duplicate: 50,
        min_latency: Duration::from_millis(1),
        max_latency: Duration::from_millis(50),
    };
    let mut world = World::new(SimConfig {
        seed,
        net,
        max_clock_skew: Duration::from_millis(250),
    });
    let server = world.add_node(Arc::new(move |env: Env| Box::pin(echo_server(env))));
    let server_addr = world.addr(server);
    let log = Arc::new(Mutex::new(Log::default()));
    let l2 = log.clone();
    let client_id = world.add_node(Arc::new(move |env: Env| {
        Box::pin(client(
            env,
            server_addr.clone(),
            l2.clone(),
            nondeterministic,
        ))
    }));

    // nemesis: its own stream of the seed, acting every 100ms of virtual time.
    let mut nem = Rng::new(seed ^ 0x6e65_6d65_7369_7300);
    let mut t = 0;
    while t < 10_000 * MS {
        t += 100 * MS;
        world.run_until(Instant(t));
        match nem.below(12) {
            0 if world.is_up(server) => world.crash(server),
            1 if !world.is_up(server) => world.restart(server),
            2 => world.block(client_id, server),
            3 => world.block(server, client_id),
            4 => world.heal(),
            _ => {}
        }
    }
    // liveness phase: faults off, everything must finish.
    world.heal();
    if !world.is_up(server) {
        world.restart(server);
    }
    world.set_net(NetConfig {
        loss: 0,
        duplicate: 0,
        min_latency: Duration::from_millis(1),
        max_latency: Duration::from_millis(5),
    });
    world.run_until(Instant(t + 60_000 * MS));
    let end_ms = world.now().0 / MS;
    let trace = world.trace_hash();
    let l = log.lock().unwrap();
    let mut errors = l.errors.clone();
    if l.completed != REQUESTS {
        errors.push(format!("liveness: {} of {REQUESTS} completed", l.completed));
    }
    Outcome {
        trace,
        completed: l.completed,
        errors,
        end_ms,
    }
}

fn arg(args: &[String], name: &str) -> Option<u64> {
    let i = args.iter().position(move |a| a == name)?;
    args.get(i + 1).and_then(move |v| v.parse().ok())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let check = args.iter().any(move |a| a == "--check-determinism");
    let nondet = args.iter().any(move |a| a == "--nondeterministic");
    let seeds: Vec<u64> = match arg(&args, "--seed") {
        Some(s) => vec![s],
        None => (0..arg(&args, "--seeds").unwrap_or(100)).collect(),
    };
    let start = std::time::Instant::now();
    let mut failed = 0;
    for &seed in &seeds {
        let a = run(seed, nondet);
        if seeds.len() == 1 {
            let (tr, c, e) = (a.trace, a.completed, a.end_ms);
            println!("seed {seed}: trace {tr:016x} completed {c} end {e}ms");
        }
        if !a.errors.is_empty() {
            failed += 1;
            println!("FAIL seed {seed}: {:?}", a.errors);
        }
        if check {
            let b = run(seed, nondet);
            if a != b {
                failed += 1;
                let (x, y) = (a.trace, b.trace);
                println!("NONDETERMINISM seed {seed}: trace {x:016x} vs {y:016x}");
            }
        }
    }
    let n = seeds.len();
    let el = start.elapsed();
    println!("{n} seeds, {failed} failures, check_determinism={check}, {el:.2?} real");
    if failed > 0 {
        std::process::exit(1);
    }
}
