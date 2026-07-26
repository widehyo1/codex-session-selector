use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_select-codex-session")
}

#[test]
fn root_help_lists_subcommands() {
    let output = Command::new(binary()).arg("--help").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("index"));
    assert!(stdout.contains("replay"));
    assert!(stdout.contains("--include-exec"));
}

#[test]
fn index_help_lists_legacy_recorder_options() {
    let output = Command::new(binary())
        .args(["index", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--sessions-root"));
    assert!(stdout.contains("--include-subsessions"));
    assert!(stdout.contains("--include-empty-messages"));
}

#[test]
fn replay_help_lists_current_replay_options() {
    let output = Command::new(binary())
        .args(["replay", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--include-exec"));
    assert!(stdout.contains("[PATH|-]"));
}

#[test]
fn version_uses_the_integrated_binary_name() {
    let output = Command::new(binary()).arg("--version").output().unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("select-codex-session {}\n", env!("CARGO_PKG_VERSION"))
    );
}
