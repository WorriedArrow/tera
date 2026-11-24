//! build and test tooling, run as `cargo xtask <command>`.

mod check;
mod deps;
mod lints;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about = "build and test tooling for tera")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
enum Command {
    /// fmt, clippy -D warnings, unit tests, dependency and lint policy, cargo-deny
    Check,
    /// run sqllogictest files and compare against tests/slt-meta
    Slt {
        /// a file or directory under tests/slt, default all
        path: Option<PathBuf>,
    },
    /// run postgres regression files and compare against tests/pgregress-meta
    Pgregress {
        /// a regression file name, default all
        name: Option<String>,
    },
    /// deterministic simulation, prints the seed of any failure
    Sim {
        /// number of seeds to run
        #[arg(long, required_unless_present = "seed", conflicts_with = "seed",
              value_parser = clap::value_parser!(u64).range(1..))]
        seeds: Option<u64>,
        /// run exactly this seed
        #[arg(long)]
        seed: Option<u64>,
        /// run only this workload, default all
        #[arg(long)]
        workload: Option<String>,
    },
    /// client library test suites against a local cluster
    Drivers,
    /// tpc-c driver with consistency checks
    Tpcc {
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        nodes: u32,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        warehouses: u32,
    },
    /// local multi-process cluster for manual runs
    Cluster {
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        nodes: u32,
    },
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Command::Check => "check",
            Command::Slt { .. } => "slt",
            Command::Pgregress { .. } => "pgregress",
            Command::Sim { .. } => "sim",
            Command::Drivers => "drivers",
            Command::Tpcc { .. } => "tpcc",
            Command::Cluster { .. } => "cluster",
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check => check::run(),
        other => Err(format!("not implemented yet: {}", other.name())),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Command, clap::Error> {
        let argv = std::iter::once("xtask").chain(args.iter().copied());
        Cli::try_parse_from(argv).map(|c| c.command)
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_every_command() {
        assert_eq!(parse(&["check"]).unwrap(), Command::Check);
        assert_eq!(parse(&["slt"]).unwrap(), Command::Slt { path: None });
        assert_eq!(
            parse(&["slt", "tests/slt/sqlite/select1.test"]).unwrap(),
            Command::Slt {
                path: Some("tests/slt/sqlite/select1.test".into())
            }
        );
        assert_eq!(
            parse(&["pgregress", "int4"]).unwrap(),
            Command::Pgregress {
                name: Some("int4".into())
            }
        );
        assert_eq!(
            parse(&["sim", "--seeds", "1000", "--workload", "bank"]).unwrap(),
            Command::Sim {
                seeds: Some(1000),
                seed: None,
                workload: Some("bank".into())
            }
        );
        assert_eq!(
            parse(&["sim", "--seed", "42"]).unwrap(),
            Command::Sim {
                seeds: None,
                seed: Some(42),
                workload: None
            }
        );
        assert_eq!(parse(&["drivers"]).unwrap(), Command::Drivers);
        assert_eq!(
            parse(&["tpcc", "--nodes", "3", "--warehouses", "10"]).unwrap(),
            Command::Tpcc {
                nodes: 3,
                warehouses: 10
            }
        );
        assert_eq!(
            parse(&["cluster", "--nodes", "3"]).unwrap(),
            Command::Cluster { nodes: 3 }
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for args in [
            &[][..],
            &["nope"],
            &["sim"],
            &["sim", "--seeds", "0"],
            &["sim", "--seeds", "10", "--seed", "1"],
            &["sim", "--seeds", "ten"],
            &["tpcc", "--nodes", "3"],
            &["tpcc", "--warehouses", "1"],
            &["cluster"],
            &["cluster", "--nodes", "0"],
            &["drivers", "extra"],
            &["check", "--fast"],
        ] {
            assert!(parse(args).is_err(), "{args:?} should not parse");
        }
    }
}
