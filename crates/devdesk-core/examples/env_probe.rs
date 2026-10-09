//! Local M1 acceptance probe; requires the owner's npm/php installations.
//! Prints only command locations, never the resolved environment or secrets.
use devdesk_core::env::ShellEnvironment;
fn main() {
    let environment = ShellEnvironment::shared();
    assert!(environment.warning.is_none(), "{:?}", environment.warning);
    let result = std::process::Command::new(&environment.shell)
        .args(["-c", "command -v npm; command -v php"])
        .env_clear()
        .envs(&environment.variables)
        .output()
        .unwrap();
    assert!(result.status.success());
    let paths = String::from_utf8(result.stdout).unwrap();
    assert_eq!(paths.lines().count(), 2);
    print!("{paths}");
    assert!(std::sync::Arc::ptr_eq(
        &environment,
        &ShellEnvironment::shared()
    ));
}
