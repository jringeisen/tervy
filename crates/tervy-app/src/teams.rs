//! Passive Teams audio activity detection. No audio is captured or inspected.
use std::ffi::{CStr, c_char, c_void};

#[derive(Default)]
pub struct Detector {
    active_samples: u8,
    quiet_samples: u8,
    notified: bool,
}

impl Detector {
    /// Sample every three seconds. Require sustained activity and 30 seconds
    /// of confirmed inactivity before allowing another reminder.
    pub fn sample(&mut self, active: Option<bool>) -> bool {
        match active {
            Some(true) => {
                self.quiet_samples = 0;
                self.active_samples = self.active_samples.saturating_add(1);
            }
            Some(false) => {
                self.active_samples = 0;
                self.quiet_samples = self.quiet_samples.saturating_add(1);
                if self.quiet_samples >= 10 {
                    self.notified = false;
                }
            }
            None => {
                // Errors cannot establish either sustained activity or a call end.
                self.active_samples = 0;
                self.quiet_samples = 0;
            }
        }
        self.active_samples >= 3 && !self.notified
    }

    pub fn call_ended(&self) -> bool {
        self.quiet_samples >= 10
    }

    pub fn acknowledge(&mut self) {
        self.notified = true;
    }
}

fn is_teams(bundle: &str) -> bool {
    ["com.microsoft.teams", "com.microsoft.teams2"]
        .iter()
        .any(|id| {
            bundle == *id
                || bundle
                    .strip_prefix(id)
                    .is_some_and(|tail| tail.starts_with('.'))
        })
}

#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}
impl Address {
    fn new(selector: &[u8; 4]) -> Self {
        Self {
            selector: u32::from_be_bytes(*selector),
            scope: u32::from_be_bytes(*b"glob"),
            element: 0,
        }
    }
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> i32;
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringGetCString(
        string: *const c_void,
        buffer: *mut c_char,
        size: isize,
        encoding: u32,
    ) -> bool;
    fn CFRelease(value: *const c_void);
}

/// Core Audio owns the object IDs; processes can disappear between queries.
/// An incomplete sample is unknown, never evidence that a meeting ended.
pub fn activity() -> Option<bool> {
    let address = Address::new(b"prs#");
    let mut size = 0;
    // SAFETY: Property addresses and output pointers have the documented ABI.
    if unsafe { AudioObjectGetPropertyDataSize(1, &address, 0, std::ptr::null(), &mut size) } != 0 {
        return None;
    }
    let mut objects = vec![0u32; size as usize / size_of::<u32>()];
    // SAFETY: The allocated buffer has the size returned by Core Audio.
    if unsafe {
        AudioObjectGetPropertyData(
            1,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            objects.as_mut_ptr().cast(),
        )
    } != 0
    {
        return None;
    }
    objects.truncate(size as usize / size_of::<u32>());
    let mut unknown = false;
    for object in objects {
        let mut bundle: *const c_void = std::ptr::null();
        let mut size = size_of_val(&bundle) as u32;
        // SAFETY: This property returns an owned CFStringRef, released below.
        let status = unsafe {
            AudioObjectGetPropertyData(
                object,
                &Address::new(b"pbid"),
                0,
                std::ptr::null(),
                &mut size,
                (&mut bundle as *mut *const c_void).cast(),
            )
        };
        if status != 0 || bundle.is_null() {
            unknown = true;
            continue;
        }
        let mut buffer = [0 as c_char; 512];
        // SAFETY: bundle is a live CFString; buffer has the supplied capacity.
        let converted = unsafe {
            CFStringGetCString(
                bundle,
                buffer.as_mut_ptr(),
                buffer.len() as isize,
                0x08000100,
            )
        };
        // SAFETY: The bundle-ID property transfers ownership to the caller.
        unsafe { CFRelease(bundle) };
        if !converted {
            unknown = true;
            continue;
        }
        // SAFETY: Successful CFStringGetCString writes a NUL-terminated string.
        let bundle = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
        if !is_teams(&bundle) {
            continue;
        }
        for selector in [b"piri", b"piro"] {
            let mut running = 0u32;
            let mut size = size_of_val(&running) as u32;
            // SAFETY: The running property returns a UInt32 into this buffer.
            let status = unsafe {
                AudioObjectGetPropertyData(
                    object,
                    &Address::new(selector),
                    0,
                    std::ptr::null(),
                    &mut size,
                    (&mut running as *mut u32).cast(),
                )
            };
            if status == 0 && running != 0 {
                return Some(true);
            }
            unknown |= status != 0;
        }
    }
    (!unknown).then_some(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_sounds_and_brief_dropouts_do_not_repeat_reminders() {
        let mut detector = Detector::default();
        assert!(!detector.sample(Some(true)));
        assert!(!detector.sample(Some(false)));
        assert!(!detector.sample(Some(true)));
        assert!(!detector.sample(Some(true)));
        assert!(detector.sample(Some(true)));
        detector.acknowledge();
        for _ in 0..9 {
            assert!(!detector.sample(Some(false)));
        }
        for _ in 0..3 {
            assert!(!detector.sample(Some(true)));
        }
        for _ in 0..10 {
            assert!(!detector.sample(Some(false)));
        }
        assert!(!detector.sample(Some(true)));
        assert!(!detector.sample(Some(true)));
        assert!(detector.sample(Some(true)));
    }

    #[test]
    fn errors_do_not_rearm_or_establish_activity() {
        let mut detector = Detector::default();
        for _ in 0..3 {
            detector.sample(Some(true));
        }
        detector.acknowledge();
        for _ in 0..20 {
            detector.sample(None);
        }
        for _ in 0..3 {
            assert!(!detector.sample(Some(true)));
        }
        let mut detector = Detector::default();
        detector.sample(Some(true));
        detector.sample(None);
        assert!(!detector.sample(Some(true)));
        assert!(!detector.sample(Some(true)));
        assert!(detector.sample(Some(true)));
    }

    #[test]
    fn only_teams_and_its_helpers_match() {
        for id in [
            "com.microsoft.teams",
            "com.microsoft.teams2",
            "com.microsoft.teams2.helper",
        ] {
            assert!(is_teams(id));
        }
        for id in [
            "com.microsoft.teams20",
            "com.google.Chrome",
            "com.microsoft.teamsfake",
        ] {
            assert!(!is_teams(id));
        }
    }
}
