//! macOS session membership and identity-checked signalling. Call on workers.
//! A process that calls setsid itself leaves our ownership scope deliberately.
use std::io;
use std::os::fd::AsRawFd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: i32,
    pub started_us: u64,
}

impl ProcessIdentity {
    /// Physical footprint in bytes, with birth checks around the kernel query.
    /// Call on a worker. A PID that exited or was reused has no valid sample.
    pub fn memory_footprint(self) -> Option<u64> {
        if Self::read(self.pid).ok() != Some(self) {
            return None;
        }
        // SAFETY: the SDK requires an owned rusage_info_v0 buffer, cast to its
        // opaque rusage_info_t pointer type. Flavor V0 fixes the structure size.
        let mut usage: libc::rusage_info_v0 = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::proc_pid_rusage(
                self.pid,
                libc::RUSAGE_INFO_V0,
                (&mut usage as *mut libc::rusage_info_v0).cast(),
            )
        };
        if result != 0 || Self::read(self.pid).ok() != Some(self) {
            return None;
        }
        Some(usage.ri_phys_footprint)
    }

    pub fn read(pid: i32) -> io::Result<Self> {
        let info = bsd_info(pid)?;
        Ok(Self {
            pid,
            started_us: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
        })
    }

    /// Recheck PID birth time immediately before signalling. The kernel does
    /// not provide an atomic identity-and-kill operation; this narrows reuse
    /// exposure and never signals a PID known to have changed identity.
    pub fn signal(self, signal: i32, sid: i32) -> io::Result<()> {
        if Self::read(self.pid).ok() != Some(self) {
            return Ok(());
        }
        // SAFETY: positive, verified process ID; getsid does not mutate state.
        if unsafe { libc::getsid(self.pid) } != sid {
            return Ok(());
        }
        // SAFETY: signal is a POSIX signal chosen by the supervisor.
        if unsafe { libc::kill(self.pid, signal) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
}

fn bsd_info(pid: i32) -> io::Result<libc::proc_bsdinfo> {
    // SAFETY: proc_pidinfo writes exactly the sized, owned SDK structure.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let result = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if result != size {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn process_ids() -> io::Result<Vec<i32>> {
    // SAFETY: null asks for the count; the following call owns its buffer.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pids = vec![0i32; count as usize + 256];
    let length = unsafe {
        libc::proc_listallpids(
            pids.as_mut_ptr().cast(),
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if length <= 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize >= pids.len() {
        return Err(io::Error::other(
            "process list changed; retry session sweep",
        ));
    }
    pids.truncate(length as usize);
    Ok(pids)
}

/// Sample every session tree from one process-list snapshot, including captured
/// background sessions and live descendants that created a separate SID. This
/// is read-only accounting, not an extension of signalling ownership. Each PID
/// contributes once, and missing/denied data is unavailable rather than zero.
pub fn memory_footprints(groups: &[Vec<OwnedSession>]) -> Vec<Option<u64>> {
    use std::collections::BTreeMap;
    let Ok(pids) = process_ids() else {
        return vec![None; groups.len()];
    };
    let records: BTreeMap<_, _> = pids
        .into_iter()
        .filter_map(|pid| {
            let info = bsd_info(pid).ok()?;
            if info.pbi_status == 5 {
                return None;
            }
            let identity = ProcessIdentity {
                pid,
                started_us: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
            };
            // SAFETY: getsid only reads kernel state for a positive process ID.
            Some((
                pid,
                (identity, info.pbi_ppid as i32, unsafe { libc::getsid(pid) }),
            ))
        })
        .collect();
    let mut owners = BTreeMap::new();
    for (group, sessions) in groups.iter().enumerate() {
        for session in sessions {
            if ProcessIdentity::read(session.leader.pid)
                .ok()
                .is_some_and(|current| current != session.leader)
            {
                continue;
            }
            for (&pid, &(identity, _, sid)) in &records {
                if sid == session.leader.pid && identity.started_us >= session.leader.started_us {
                    owners.entry(pid).or_insert(group);
                }
            }
        }
    }
    loop {
        let mut added = false;
        for (&pid, &(identity, parent, _)) in &records {
            if owners.contains_key(&pid) {
                continue;
            }
            if let Some(&group) = owners.get(&parent)
                && let Some(&(ancestor, _, _)) = records.get(&parent)
                && ancestor.started_us <= identity.started_us
                && ProcessIdentity::read(parent).ok() == Some(ancestor)
            {
                owners.insert(pid, group);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut totals = vec![None; groups.len()];
    let mut unavailable = vec![false; groups.len()];
    for (pid, group) in owners {
        match records[&pid].0.memory_footprint() {
            Some(bytes) => {
                totals[group] = Some(totals[group].unwrap_or(0u64).saturating_add(bytes))
            }
            None => unavailable[group] = true,
        }
    }
    for (total, unavailable) in totals.iter_mut().zip(unavailable) {
        if unavailable {
            *total = None;
        }
    }
    totals
}

#[derive(Clone, Copy, Debug)]
pub struct OwnedSession {
    pub leader: ProcessIdentity,
}

impl OwnedSession {
    pub fn capture(pid: u32) -> io::Result<Self> {
        let leader = ProcessIdentity::read(pid as i32)?;
        // SAFETY: the child must be the session leader created by tty::new.
        let sid = unsafe { libc::getsid(leader.pid) };
        if sid < 0 {
            return Err(io::Error::last_os_error());
        }
        if sid != leader.pid {
            return Err(io::Error::other("PTY child is not its session leader"));
        }
        Ok(Self { leader })
    }

    /// Includes background jobs in other process groups, even without a live
    /// leader. A reused SID with a different leader fails closed. Zombies
    /// cannot hold ports or execute and are left to their parent to reap.
    pub fn members(self) -> io::Result<Vec<ProcessIdentity>> {
        if let Ok(current) = ProcessIdentity::read(self.leader.pid)
            && current != self.leader
        {
            return Ok(Vec::new());
        }
        let pids = process_ids()?;
        let mut members = Vec::new();
        for &pid in &pids {
            if pid <= 0 || unsafe { libc::getsid(pid) } != self.leader.pid {
                continue;
            }
            if let Ok(info) = bsd_info(pid) {
                let identity = ProcessIdentity {
                    pid,
                    started_us: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
                };
                if info.pbi_status != 5 && identity.started_us >= self.leader.started_us {
                    members.push(identity);
                }
            }
        }
        Ok(members)
    }

    /// Explicit opt-in for a CLI which creates separate sessions for its tool
    /// jobs. Discover only while its original leader is alive; never infer
    /// ownership from a process name or a reused PID. Call before signalling
    /// the parent, then retain and journal the captured birth identities.
    pub fn descendant_sessions(self) -> io::Result<Vec<Self>> {
        use std::collections::{BTreeMap, BTreeSet};
        if ProcessIdentity::read(self.leader.pid)? != self.leader {
            return Err(io::Error::other("Session leader identity changed"));
        }
        let mut records = BTreeMap::new();
        for pid in process_ids()? {
            if let Ok(info) = bsd_info(pid) {
                let identity = ProcessIdentity {
                    pid,
                    started_us: info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec,
                };
                if info.pbi_status != 5 && identity.started_us >= self.leader.started_us {
                    records.insert(pid, (info.pbi_ppid as i32, identity));
                }
            }
        }
        let mut descendants = BTreeSet::from([self.leader.pid]);
        loop {
            let mut added = false;
            for (&pid, &(parent, child)) in &records {
                if descendants.contains(&pid) || !descendants.contains(&parent) {
                    continue;
                }
                let Some(&(_, ancestor)) = records.get(&parent) else {
                    continue;
                };
                if ancestor.started_us <= child.started_us
                    && ProcessIdentity::read(parent).ok() == Some(ancestor)
                {
                    descendants.insert(pid);
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        let mut sessions = Vec::new();
        for pid in descendants {
            if pid == self.leader.pid || unsafe { libc::getsid(pid) } != pid {
                continue;
            }
            if let Ok(session) = Self::capture(pid as u32)
                && records
                    .get(&pid)
                    .is_some_and(|(_, identity)| *identity == session.leader)
            {
                sessions.push(session);
            }
        }
        Ok(sessions)
    }

    /// Signal foreground and leader groups while their identity is known,
    /// then sweep individual members so background job groups are covered.
    pub fn signal(self, signal: i32, master: &std::fs::File) -> io::Result<()> {
        let members = self.members()?;
        let foreground = unsafe { libc::tcgetpgrp(master.as_raw_fd()) };
        let mut signalled_groups = Vec::new();
        for group in [foreground, self.leader.pid] {
            if group <= 0 || signalled_groups.contains(&group) {
                continue;
            }
            if let Some(member) = members.iter().find(|member| member.pid == group)
                && ProcessIdentity::read(group).ok() == Some(*member)
                && unsafe { libc::getsid(group) } == self.leader.pid
            {
                // SAFETY: the group leader still belongs to our session.
                if unsafe { libc::killpg(group, signal) } == 0 {
                    signalled_groups.push(group);
                }
            }
        }
        for member in members {
            // Do not deliver SIGINT (or another graceful signal) twice to
            // members already reached by a group signal.
            if signalled_groups.contains(&unsafe { libc::getpgid(member.pid) }) {
                continue;
            }
            member.signal(signal, self.leader.pid)?;
        }
        Ok(())
    }

    pub fn foreground_job(self, master: &std::fs::File) -> Option<String> {
        let foreground = unsafe { libc::tcgetpgrp(master.as_raw_fd()) };
        if foreground <= 0 || foreground == self.leader.pid {
            return None;
        }
        let member = self
            .members()
            .ok()?
            .into_iter()
            .find(|member| unsafe { libc::getpgid(member.pid) } == foreground)?;
        let info = bsd_info(member.pid).ok()?;
        let bytes: Vec<u8> = info
            .pbi_comm
            .iter()
            .take_while(|&&byte| byte != 0)
            .map(|&byte| byte as u8)
            .collect();
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Read a live shell's cwd through the macOS SDK, guarding PID reuse before
/// and after the query. This belongs on the supervisor, never in a GPUI render.
pub fn cwd(identity: ProcessIdentity) -> io::Result<std::path::PathBuf> {
    if ProcessIdentity::read(identity.pid)? != identity {
        return Err(io::Error::other("Shell identity changed"));
    }
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let count = unsafe {
        libc::proc_pidinfo(
            identity.pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast(),
            size,
        )
    };
    if count != size {
        return Err(io::Error::last_os_error());
    }
    if ProcessIdentity::read(identity.pid)? != identity {
        return Err(io::Error::other("Shell identity changed"));
    }
    use std::os::unix::ffi::OsStringExt;
    let bytes = info
        .pvi_cdir
        .vip_path
        .iter()
        .flatten()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect::<Vec<_>>();
    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(bytes));
    if !path.is_absolute() {
        return Err(io::Error::other("Shell cwd unavailable"));
    }
    Ok(path)
}
