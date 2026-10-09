//! Cleanup guard for opt-in GUI probes. IDs and birth times come only from
//! that probe's ownership log, never from a process-name match.
use devdesk_term::kill::{OwnedSession, ProcessIdentity};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let owned = OwnedSession {
        leader: ProcessIdentity {
            pid: args[1].parse().expect("positive SID"),
            started_us: args[2].parse().expect("birth time"),
        },
    };
    assert!(owned.leader.pid > 0);
    let members = owned.members().expect("enumerate probe session");
    println!("{}", members.len());
    for member in members {
        member
            .signal(libc::SIGKILL, owned.leader.pid)
            .expect("stop probe survivor");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !owned.members().expect("verify cleanup").is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "probe survivor could not be killed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
