//! every command without an implementation must fail, not report success.

use std::process::Command;

#[test]
fn stub_commands_exit_non_zero() {
    let cases: &[(&[&str], &str)] = &[
        (&["slt"], "slt"),
        (&["slt", "tests/slt"], "slt"),
        (&["pgregress", "int4"], "pgregress"),
        (&["sim", "--seeds", "10"], "sim"),
        (&["sim", "--seed", "7", "--workload", "register"], "sim"),
        (&["drivers"], "drivers"),
        (&["tpcc", "--nodes", "3", "--warehouses", "1"], "tpcc"),
        (&["cluster", "--nodes", "3"], "cluster"),
    ];
    for (args, name) in cases {
        let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(*args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(&format!("not implemented yet: {name}")),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn bad_arguments_exit_non_zero() {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["sim"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
