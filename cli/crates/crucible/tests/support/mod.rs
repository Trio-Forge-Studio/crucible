use serde_json::Value;
use std::process::{Command, Output};
pub const KEY: &str = "sentinel-Key_+/%=NeverPrint";
pub fn root(origin: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_crucible"));
    cmd.current_dir(env!("CARGO_TARGET_TMPDIR")).env("CRUCIBLE_OC_ORIGIN", origin).env("CRUCIBLE_OC_API_KEY", KEY);
    cmd
}
pub fn command(origin: &str) -> Command {
    let mut cmd = root(origin);
    cmd.args(["oc", "--poll-interval", "0", "--retry-delay", "0"]);
    cmd
}
pub fn whoami(origin: &str) -> Command {
    let mut cmd = root(origin);
    cmd.arg("whoami");
    cmd
}
pub fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout must be one JSON document, got {:?}, stderr {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
pub fn code(output: &Output) -> i32 {
    output.status.code().expect("binary exits normally")
}
