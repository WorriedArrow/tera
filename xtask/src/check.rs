use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{deps, lints};

pub fn run() -> Result<(), String> {
    let root = workspace_root();
    // look for cargo-deny first so a missing install fails in seconds, not
    // after the test run.
    if !cargo(&root, &["deny", "--version"]).quiet() {
        return Err("cargo-deny is not installed. install it with \
                    `cargo install --locked cargo-deny` and rerun"
            .into());
    }
    cargo(&root, &["fmt", "--all", "--", "--check"]).step("fmt")?;
    lints::check(&root)?;
    cargo(
        &root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )
    .step("clippy")?;
    cargo(&root, &["test", "--workspace", "--all-features"]).step("test")?;
    deps::check(&root)?;
    cargo(&root, &["deny", "check"]).step("cargo-deny")?;
    eprintln!("check passed");
    Ok(())
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

pub fn cargo(root: &Path, args: &[&str]) -> Step {
    // cargo sets CARGO for the processes it runs, so `cargo xtask` uses the
    // same toolchain for every step.
    let bin = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(bin);
    cmd.args(args).current_dir(root);
    Step(cmd)
}

pub struct Step(Command);

impl Step {
    pub fn step(mut self, name: &str) -> Result<(), String> {
        eprintln!("== {name}");
        let status = self
            .0
            .status()
            .map_err(|e| format!("{name}: could not run cargo: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{name} failed"))
        }
    }

    pub fn output(mut self, name: &str) -> Result<String, String> {
        let out = self
            .0
            .output()
            .map_err(|e| format!("{name}: could not run cargo: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "{name} failed:\n{}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        String::from_utf8(out.stdout).map_err(|e| format!("{name}: output is not utf-8: {e}"))
    }

    fn quiet(mut self) -> bool {
        self.0.output().map(|o| o.status.success()).unwrap_or(false)
    }
}
