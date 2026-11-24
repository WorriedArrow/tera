//! keeps the lint setup that enforces the AGENTS.md code rules from eroding:
//! unsafe code only in tr_storage, tr_exec and tr_env, every other lint the
//! same in every crate, and the root clippy.toml determinism bans in force for
//! core crates.

use std::path::Path;

use crate::deps::CORE;

const UNSAFE_ALLOWED: &[&str] = &["tr_env", "tr_storage", "tr_exec"];

pub fn check(root: &Path) -> Result<(), String> {
    eprintln!("== lint config");
    let workspace = read(&root.join("Cargo.toml"))?;
    let mut expected_rust = section(&workspace, "workspace.lints.rust")
        .ok_or("Cargo.toml has no [workspace.lints.rust]")?;
    let expected_clippy = section(&workspace, "workspace.lints.clippy")
        .ok_or("Cargo.toml has no [workspace.lints.clippy]")?;
    let forbid = "unsafe_code=\"forbid\"".to_string();
    if !expected_rust.contains(&forbid) {
        return Err("[workspace.lints.rust] must set unsafe_code = \"forbid\"".into());
    }
    expected_rust.retain(|l| *l != forbid);

    let mut errors = Vec::new();
    for (name, dir) in members(root)? {
        let manifest = read(&dir.join("Cargo.toml"))?;
        if UNSAFE_ALLOWED.contains(&name.as_str()) {
            if section(&manifest, "lints.rust").as_ref() != Some(&expected_rust)
                || section(&manifest, "lints.clippy").as_ref() != Some(&expected_clippy)
            {
                errors.push(format!(
                    "{name}: [lints.rust] and [lints.clippy] must match [workspace.lints] \
                     without unsafe_code"
                ));
            }
        } else if section(&manifest, "lints") != Some(vec!["workspace=true".to_string()]) {
            errors.push(format!("{name}: must use [lints] workspace = true"));
        }
        if CORE.contains(&name.as_str()) {
            for file in ["clippy.toml", ".clippy.toml"] {
                if dir.join(file).exists() {
                    errors.push(format!(
                        "{name}: {file} would replace the root clippy.toml determinism bans"
                    ));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("lint config:\n  {}", errors.join("\n  ")))
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// every crate under crates/ plus xtask, named by directory.
fn members(root: &Path) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let crates = root.join("crates");
    let entries = std::fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))?;
    let mut out = vec![("xtask".to_string(), root.join("xtask"))];
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.join("Cargo.toml").is_file() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("{}: name is not utf-8", path.display()))?;
            out.push((name.to_string(), path.clone()));
        }
    }
    out.sort();
    Ok(out)
}

/// the `key = value` lines of a toml table, with whitespace and comment lines
/// dropped and sorted, so formatting doesn't count as drift. only handles the
/// one-line entries the manifests here use.
fn section(text: &str, name: &str) -> Option<Vec<String>> {
    let header = format!("[{name}]");
    let mut lines = text.lines().map(str::trim);
    lines.find(|l| *l == header)?;
    let mut entries: Vec<String> = lines
        .take_while(|l| !l.starts_with('['))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().collect())
        .collect();
    entries.sort();
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_ignores_formatting_and_order() {
        let text = "[a]\nx = 1\n\n[lints.clippy]\n# why\nb_lint = \"deny\"\na_lint   =  \"warn\"\n\n[c]\ny = 2\n";
        assert_eq!(
            section(text, "lints.clippy"),
            Some(vec!["a_lint=\"warn\"".into(), "b_lint=\"deny\"".into()])
        );
        assert_eq!(section(text, "lints.rust"), None);
        assert_eq!(section("[lints.rust]\n", "lints.rust"), Some(vec![]));
    }
}
