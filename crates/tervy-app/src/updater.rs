//! The Mac bundle loads its Swift/Sparkle bridge at runtime. CLI and smoke
//! builds do not contain that bundle, so update controls stay unavailable.
use std::ffi::{CStr, CString, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub(crate) enum UpdateEvent {
    Available(String),
    Current,
    Error(String),
    InstallRequested,
}

type Start = unsafe extern "C" fn(Option<extern "C" fn(i32, *const c_char)>) -> i32;
type Action = unsafe extern "C" fn();

struct Bridge {
    _library: *mut c_void,
    start: Start,
    check: Action,
    resume_install: Action,
}

// The library is retained for the process lifetime; Sparkle is main-thread-only.
unsafe impl Send for Bridge {}
unsafe impl Sync for Bridge {}

static BRIDGE: OnceLock<Option<Bridge>> = OnceLock::new();
static EVENTS: OnceLock<async_channel::Sender<UpdateEvent>> = OnceLock::new();
static AVAILABLE: AtomicBool = AtomicBool::new(false);

extern "C" fn receive_event(kind: i32, value: *const c_char) {
    let value = if value.is_null() {
        String::new()
    } else {
        // The Swift bridge keeps the C string alive for this callback.
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    };
    let event = match kind {
        0 => UpdateEvent::Current,
        1 => UpdateEvent::Available(value),
        2 => UpdateEvent::Error(value),
        3 => UpdateEvent::InstallRequested,
        _ => return,
    };
    if let Some(sender) = EVENTS.get() {
        let _ = sender.try_send(event);
    }
}

unsafe fn symbol<T>(library: *mut c_void, name: &'static CStr) -> Option<T> {
    let symbol = unsafe { libc::dlsym(library, name.as_ptr()) };
    (!symbol.is_null()).then(|| unsafe { std::mem::transmute_copy(&symbol) })
}

fn load_bridge() -> Option<Bridge> {
    let executable = std::env::current_exe().ok()?;
    let path = executable
        .parent()?
        .parent()?
        .join("Frameworks/TervyUpdater.dylib");
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let library = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW) };
    if library.is_null() {
        tracing::warn!("Could not load Tervy updater bridge");
        return None;
    }
    let symbols = unsafe {
        Some((
            symbol(library, c"tervy_updater_start")?,
            symbol(library, c"tervy_updater_check")?,
            symbol(library, c"tervy_updater_resume_install")?,
        ))
    };
    let Some((start, check, resume_install)) = symbols else {
        unsafe { libc::dlclose(library) };
        return None;
    };
    Some(Bridge {
        _library: library,
        start,
        check,
        resume_install,
    })
}

pub(crate) fn start(sender: async_channel::Sender<UpdateEvent>) -> bool {
    let Some(bridge) = BRIDGE.get_or_init(load_bridge) else {
        return false;
    };
    let _ = EVENTS.set(sender);
    let available = unsafe { (bridge.start)(Some(receive_event)) } == 1;
    AVAILABLE.store(available, Ordering::Relaxed);
    available
}

pub(crate) fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

pub(crate) fn check() {
    if available()
        && let Some(Some(bridge)) = BRIDGE.get()
    {
        unsafe { (bridge.check)() };
    }
}

pub(crate) fn resume_install() {
    if available()
        && let Some(Some(bridge)) = BRIDGE.get()
    {
        unsafe { (bridge.resume_install)() };
    }
}
