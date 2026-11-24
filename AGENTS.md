# tera

tera is a distributed sql database written in rust. it speaks the postgres wire protocol and the postgres sql dialect, stores data in its own lsm engine, replicates with raft over ranges of a sorted key space, and runs serializable distributed transactions. every layer can run inside a deterministic simulator. license is mit or apache-2.0 at your option.

this file is the only entry point for every agent and human. read all of it before doing anything.

## rules

- the repo is the only context. anything needed to pick up a task cold lives in the repo or in its github issues and prs. no agent relies on anything else.
- no memory systems, no notes outside the repo, no CLAUDE.md. .claude/ holds only pointers to this file.
- nothing operator-specific goes in the repo: no local paths, hostnames, usernames, keys or personal config.
- nobody edits AGENTS.md. agents don't write prose docs about what they built: no architecture writeups, no readme sections, no "how it works" files. if the design below needs changing, open a `kind:design-change` issue.
- comments say why and cite the paper, rfc or postgres behavior they follow. they don't narrate the code.
- every change goes through a pr and the verification section below. nobody pushes to main.
- write plainly and in lowercase. no marketing words.

## code rules

- rust stable, edition 2024. `cargo xtask check` passes with zero warnings.
- determinism: core crates (tr_storage, tr_mvcc, tr_raft, tr_kv, tr_sql, tr_opt, tr_exec) never touch time, threads, randomness, the filesystem or the network directly. all of it goes through tr_env. clippy disallowed-methods and disallowed-types enforce this, including a ban on std HashMap and HashSet with random state in core crates (use the deterministic maps tr_env exports).
- `unsafe` only in tr_storage (block cache, mmap), tr_exec (vector kernels) and tr_env. every block gets a `// safety:` comment.
- nothing a client sends may panic or crash a node. bad input gets a postgres error with the right sqlstate.
- never special-case a test: no branches on test names, slt file paths, seeds, or query text from a test. one instance gets the pr rejected and audited.
- unimplemented features return sqlstate 0A000 (feature_not_supported) with the feature named. never fake success.
- dependencies only from the allowed list below. anything else is a `kind:design-change` issue.
- don't copy code from other databases. reading postgres, cockroachdb, tidb, foundationdb or tigerbeetle to understand an approach is fine and researchers cite them. cockroachdb's source is under a non-open license now, so read its design docs and papers, not its code.

## allowed dependencies

tokio and tokio-util (tr_env's production impl and tr_pgwire only), bytes, serde, postcard, crc32c, xxhash-rust, zstd, lz4_flex, rustls, sha2, hmac, pbkdf2, base64, tracing, tracing-subscriber, clap, thiserror, anyhow (xtask only), smallvec, arrayvec, parking_lot (outside core crates), crossbeam (outside core crates), tikv-jemallocator, proptest, criterion. dev-dependencies for compatibility tests: tokio-postgres, sqlx. license check through cargo-deny: mit, apache-2.0, bsd, isc, zlib, unicode, mpl-2.0 only.

built in tera, never pulled from crates: the sql parser, the lsm engine, raft, the pgwire protocol, numeric arithmetic, every sql type and function, the optimizer, the executor, the simulator, the slt runner, the history checkers.

## crates

| crate | does |
|---|---|
| tr_env | time, randomness, network, disk and task spawning behind traits, with a production impl (tokio, real io) and a simulated one |
| tr_sim | deterministic cluster simulator: single-threaded executor, simulated network (latency, loss, reordering, duplication, partitions), simulated disks (latency, lost unsynced writes on crash, torn writes, corruption), clock skew, node crash and restart, workload drivers, invariant checkers |
| tr_check | history checkers: linearizability for single-key registers, elle-style cycle detection on list-append histories for serializability |
| tr_storage | lsm engine: wal, memtable, sstables with block index and bloom filters, leveled compaction, block cache, checksums on every block, crash recovery |
| tr_mvcc | mvcc over tr_storage with hybrid logical clock timestamps, write intents, gc of old versions |
| tr_raft | raft with pre-vote, check quorum, leader leases, joint consensus membership changes, snapshots, log truncation, and multi-raft batching for thousands of groups per node |
| tr_kv | ranges over the key space, a meta range of range descriptors, leaseholders, range split, merge and rebalance, node liveness, distributed transactions with transaction records and parallel commits, serializable isolation with read refresh, closed timestamps |
| tr_sql_parser | hand-written recursive descent parser for the postgres dialect, postgres error positions and messages |
| tr_sql | binder, catalog stored in kv, the postgres type system (int2/4/8, numeric, float4/8, text, varchar, char, bytea, bool, date, time, timestamp, timestamptz, interval, uuid, json, jsonb, arrays, enums), functions and operators with postgres semantics, online schema changes |
| tr_opt | logical plans, normalization rules, then a cascades cost-based optimizer with table statistics and histograms |
| tr_exec | vectorized executor on column batches, every operator, distributed flows across nodes |
| tr_pgwire | protocol v3 server: startup, scram-sha-256 and trust auth, tls via rustls, simple and extended query, portals, copy in and out, cancel requests, notices, every error field |
| tr | the `tera` binary: `tera start`, `tera init`, `tera join`, `tera sql` (a thin client) |
| xtask | build and test tooling |

## commands

```
cargo xtask check                 fmt, clippy -D warnings, unit tests, cargo-deny
cargo xtask slt [path]            sqllogictest files, compared against tests/slt-meta
cargo xtask pgregress [name]      postgres regression files, compared against tests/pgregress-meta
cargo xtask sim --seeds N [--seed S] [--workload W]   deterministic simulation, prints the seed of any failure
cargo xtask drivers               client library test suites against a local cluster
cargo xtask tpcc --nodes N --warehouses W   tpc-c driver built in xtask
cargo xtask cluster --nodes N     local multi-process cluster for manual runs
```

building any command that doesn't exist yet is part of m0.

## oracles

- **sqllogictest**: sqlite's public-domain slt corpus vendored under tests/slt/sqlite, at a pinned version. the in-house runner supports the whole slt format (statement ok/error, query with type strings, rowsort, valuesort, hash-threshold, skipif, onlyif). tera answers to the engine name `postgresql`. expectations live in tests/slt-meta, and any result that differs from them, including an unexpected pass, fails ci.
- **postgres regression suite**: src/test/regress/sql and expected/ from the latest postgres release, vendored under tests/pgregress with the postgresql license file. per-file expectations in tests/pgregress-meta record pass, or fail with the number of differing lines, and any change fails ci until expectations move in the same pr.
- **drivers**: tokio-postgres and sqlx test suites in ci on every pr touching tr_pgwire or tr_sql. psycopg, node-postgres and pgjdbc test suites nightly.
- **simulation**: every pr touching tr_env, tr_storage, tr_mvcc, tr_raft or tr_kv runs 1000 seeds across all workloads. nightly runs a million. invariants: acknowledged writes survive any crash, no two raft leaders in one term, committed raft entries never change, single-key operations linearize, list-append histories have no serializability cycles, ranges never overlap or leave gaps, a node that restarts recovers to a consistent state. a failing seed becomes a `kind:bug` issue with the seed and workload, and `cargo xtask sim --seed S` reproduces it bit for bit.
- **tpc-c**: the in-house driver checks the spec's consistency conditions after every run, on 1 node and on 3 nodes with one node killed mid-run.

## verification

1. ci: `cargo xtask check`, plus the slt, pgregress, driver and sim runs mapped to the touched crates.
2. review: a fresh reviewer agent that wrote none of the code gets the issue, the diff and the developer report. it walks touched algorithms against the cited paper or postgres behavior, reruns every command in the report, examines expectation changes (a big jump in passes from a small diff gets spot-checked test by test), and checks every code rule above. it writes a verdict and never suggests fixes.
3. a failed review never goes back to its author. the orchestrator spawns a new developer with the issue, the branch and the verdict. after three failed rounds, the orchestrator splits the issue or opens a research issue about the blocker.
4. the orchestrator squash-merges once the verdict passes and ci is green. if main goes red, every slot stops and the fix is the only active issue.
5. before checking off a milestone, a fresh reviewer audits ten random merged prs from it. findings become bugs that block the milestone.

## protocol

labels: `state:ready|in-progress|review|failed-review|blocked|done`, `kind:feature|bug|research|design-change|triage`, `m0` to `m6`, `crate:<name>` for every crate an issue may touch, `owner-review`. the orchestrator never runs two issues with overlapping crate labels at once, except tr_sql and tr_exec work that touches different files the issue names.

issue body: goal, references (papers, rfcs, postgres docs and source paths), scope (crates and files), acceptance (exact slt files, pgregress files, sim workloads and seed counts, unit tests), out of scope, prior attempts.

pr body: `closes #n`, what changed per crate, every command run with its result, expectation changes (fail to pass as a count, every pass to fail with its reason), questions for the orchestrator.

review verdict: `verdict: pass|fail`, then issues as `[blocking|minor] path:line, what's wrong, which rule or reference it breaks`, then the commands the reviewer reran with output.

## roles

**orchestrator**: plans milestones into issues, keeps 8 agents running, runs verification, merges, maintains the pinned tracking issue per milestone. writes no product code.

**developer**: reads the issue, its references and the code in scope. implements with unit tests. runs `cargo xtask check` and every acceptance run, updates expectations only for tests the change affects, opens the pr. stays inside the scoped crates. after two failed attempts at the same problem, marks the issue blocked with what was tried.

**reviewer**: see verification step 2. never writes code.

**researcher**: answers one research issue with a recommendation, evidence (papers, postgres source paths, how at least two other systems do it), rejected options with reasons, and what the evidence doesn't settle. prototypes go on `research/<n>` branches that never merge.

**triage**: runs a suite, groups failures by root cause, files one issue per cause listing every affected test, sorted with the biggest cause first. adds to existing issues when the cause matches.

## owner decisions

changes to the crate layout, the determinism rules, the transaction model, the storage format after m1, dependencies outside the list, and licensing. agents pick the most conservative option, label the issue `owner-review`, write down the rejected options, and keep working.

## milestones

check items off in the tracking issue as they merge. targets are minimums.

### m0: skeleton
- workspace with every crate, xtask with every command (stubs where the subsystem doesn't exist), license files copied verbatim from upstream
- ci on linux and macos: check, cargo-deny, clippy determinism config
- tr_env traits with both impls, tr_sim executor running a trivial two-node echo workload with network faults
- slt runner, pgregress runner, both corpora vendored, expectations generated
- tr_pgwire handshake so psql connects and gets a feature_not_supported error

### m1: single node sql
- tr_storage complete with crash recovery under the simulated disk, 10,000 seeds clean
- tr_mvcc on tr_storage
- parser for select, insert, update, delete, create and drop table, create index, transactions
- catalog, binder, core types, heuristic planner, vectorized executor with scans, filters, projections, sorts, limits, hash and nested loop joins, hash aggregation
- extended query protocol, prepared statements, copy
- 100% of slt select1 through select5, psql and tokio-postgres work for basic crud

### m2: full single node sql
- snapshot and serializable isolation on one node, sequences, defaults, constraints (primary, unique, foreign key, check, not null), secondary indexes
- subqueries (correlated included), ctes (recursive included), window functions, set operations, merge join, every aggregate in postgres's core
- numeric with postgres semantics, timestamps and intervals, jsonb with operators and gin-style indexing later, arrays
- 95% of the slt corpus, 40% of pgregress files passing
- sqlx test suite passing

### m3: distribution
- hlc, tr_raft with every feature in the crate table, multi-raft
- ranges, meta range, leaseholders, splits, merges, rebalancing, node liveness
- distributed transactions with parallel commits and read refresh
- 3 and 5 node simulated clusters pass every invariant over 100,000 seeds with partitions, crashes and clock skew
- tpc-c with 3 nodes passes consistency checks with one node killed mid-run

### m4: optimizer and distributed execution
- table statistics and histograms, cascades optimizer, join ordering
- distributed flows: filter, projection and partial aggregation pushed to leaseholders
- online schema changes (add column, add index, drop column) under load in simulation
- 60% of pgregress files, psycopg and node-postgres suites passing

### m5: operations
- backup and restore to a directory, changefeeds over a pgwire replication-style stream
- metrics endpoint, slow query log, explain analyze with per-node timing
- tls everywhere, scram auth, roles and grants

### m6: performance
- tpc-c and tpc-h comparisons against postgres on one node and against tera's own previous runs on 3 nodes, tracked in a nightly issue
- profile-driven work on the slowest ten tpc-h queries

### forever
after m6 nothing stops. the orchestrator loops on: triage every suite and fix the biggest root cause, raise pgregress and driver pass rates, add sim workloads and invariants (bank transfers, secondary index consistency, schema changes under partitions), add postgres features in order of how many pgregress files they unblock, and performance work from profiles.
