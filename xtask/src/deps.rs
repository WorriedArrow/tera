//! direct dependency allowlist from AGENTS.md. cargo-deny can only allow or
//! deny crates across the whole graph, which would mean listing every
//! transitive dependency, so the rule about what workspace crates may depend
//! on directly is checked here against `cargo tree`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::check::cargo;

/// crates that must stay deterministic (AGENTS.md, code rules).
pub const CORE: &[&str] = &[
    "tr_storage",
    "tr_mvcc",
    "tr_raft",
    "tr_kv",
    "tr_sql",
    "tr_opt",
    "tr_exec",
];

const ALLOWED: &[&str] = &[
    "tokio",
    "tokio-util",
    "bytes",
    "serde",
    "postcard",
    "crc32c",
    "xxhash-rust",
    "zstd",
    "lz4_flex",
    "rustls",
    "sha2",
    "hmac",
    "pbkdf2",
    "base64",
    "tracing",
    "tracing-subscriber",
    "clap",
    "thiserror",
    "anyhow",
    "smallvec",
    "arrayvec",
    "parking_lot",
    "crossbeam",
    "tikv-jemallocator",
    "proptest",
    "criterion",
];

const DEV_ONLY: &[&str] = &["tokio-postgres", "sqlx"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Normal,
    Build,
    Dev,
}

impl Kind {
    fn edge(self) -> &'static str {
        match self {
            Kind::Normal => "normal",
            Kind::Build => "build",
            Kind::Dev => "dev",
        }
    }
}

pub fn check(root: &Path) -> Result<(), String> {
    eprintln!("== dependency allowlist");
    let mut errors = Vec::new();
    for kind in [Kind::Normal, Kind::Build, Kind::Dev] {
        let tree = cargo(
            root,
            &[
                "tree",
                "--workspace",
                "--all-features",
                "--target",
                "all",
                "--no-dedupe",
                "--depth",
                "1",
                "--prefix",
                "depth",
                "--edges",
                kind.edge(),
                "--format",
                "{p}",
            ],
        )
        .output("cargo tree")?;
        let deps = parse_tree(&tree)?;
        for (krate, direct) in &deps {
            for dep in direct.iter().filter(|d| !deps.contains_key(*d)) {
                if let Err(why) = allowed(krate, dep, kind) {
                    errors.push(format!(
                        "{krate} has {} dependency {dep}: {why}",
                        kind.edge()
                    ));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "dependencies outside the AGENTS.md allowlist:\n  {}",
            errors.join("\n  ")
        ))
    }
}

/// maps each workspace member to its direct dependencies, from `cargo tree
/// --prefix depth --depth 1` output where depth 0 lines are members.
fn parse_tree(out: &str) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in out.lines() {
        let digits = line.bytes().take_while(u8::is_ascii_digit).count();
        // blank separators between roots and `[dev-dependencies]` style headers
        if digits == 0 {
            continue;
        }
        let name = line[digits..]
            .split_whitespace()
            .next()
            .ok_or_else(|| format!("unexpected cargo tree line: {line}"))?
            .to_string();
        match &line[..digits] {
            "0" => {
                deps.entry(name.clone()).or_default();
                current = Some(name);
            }
            "1" => {
                let member = current
                    .as_ref()
                    .ok_or_else(|| format!("dependency before any member: {line}"))?;
                deps.get_mut(member)
                    .expect("member inserted at depth 0")
                    .insert(name);
            }
            _ => return Err(format!("unexpected depth in cargo tree line: {line}")),
        }
    }
    Ok(deps)
}

fn allowed(krate: &str, dep: &str, kind: Kind) -> Result<(), &'static str> {
    if DEV_ONLY.contains(&dep) {
        return if kind == Kind::Dev {
            Ok(())
        } else {
            Err("allowed only as a dev-dependency for compatibility tests")
        };
    }
    if !ALLOWED.contains(&dep) {
        return Err("not on the allowed list, open a kind:design-change issue");
    }
    match dep {
        "tokio" | "tokio-util" if !matches!(krate, "tr_env" | "tr_pgwire") => {
            Err("allowed only in tr_env and tr_pgwire")
        }
        "anyhow" if krate != "xtask" => Err("allowed only in xtask"),
        "parking_lot" | "crossbeam" if CORE.contains(&krate) => Err("not allowed in core crates"),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cargo_tree_depth_output() {
        let out = "0tr v0.1.0 (/w/crates/tr)\n\
                   1tr_env v0.1.0 (/w/crates/tr_env)\n\
                   \n\
                   0tr_env v0.1.0 (/w/crates/tr_env)\n\
                   \n\
                   0xtask v0.1.0 (/w/xtask)\n\
                   1clap v4.5.0\n";
        let deps = parse_tree(out).unwrap();
        assert_eq!(deps.len(), 3);
        assert_eq!(deps["tr"], BTreeSet::from(["tr_env".to_string()]));
        assert!(deps["tr_env"].is_empty());
        assert_eq!(deps["xtask"], BTreeSet::from(["clap".to_string()]));
        assert!(parse_tree("1clap v4.5.0\n").is_err());
        assert!(parse_tree("0xtask v0.1.0\n2clap v4.5.0\n").is_err());
    }

    #[test]
    fn applies_agents_md_rules() {
        assert!(allowed("xtask", "clap", Kind::Normal).is_ok());
        assert!(allowed("tr_storage", "crc32c", Kind::Normal).is_ok());
        assert!(allowed("tr_storage", "rocksdb", Kind::Normal).is_err());
        assert!(allowed("tr_sql_parser", "sqlparser", Kind::Dev).is_err());

        assert!(allowed("tr_env", "tokio", Kind::Normal).is_ok());
        assert!(allowed("tr_pgwire", "tokio-util", Kind::Normal).is_ok());
        assert!(allowed("tr_kv", "tokio", Kind::Normal).is_err());
        assert!(allowed("tr_sim", "tokio", Kind::Dev).is_err());

        assert!(allowed("xtask", "anyhow", Kind::Normal).is_ok());
        assert!(allowed("tr_sql", "anyhow", Kind::Normal).is_err());

        assert!(allowed("tr_pgwire", "parking_lot", Kind::Normal).is_ok());
        assert!(allowed("tr_raft", "parking_lot", Kind::Normal).is_err());
        assert!(allowed("tr_exec", "crossbeam", Kind::Normal).is_err());

        assert!(allowed("tr_pgwire", "tokio-postgres", Kind::Dev).is_ok());
        assert!(allowed("tr_pgwire", "tokio-postgres", Kind::Normal).is_err());
        assert!(allowed("tr", "sqlx", Kind::Build).is_err());
    }
}
