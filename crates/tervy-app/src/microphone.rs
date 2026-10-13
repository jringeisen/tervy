//! Request microphone access from Tervy's main app, which macOS treats as the
//! responsible process for its meeting audio helper.

use block2::RcBlock;
use objc2::msg_send;
use objc2::runtime::{AnyClass, Bool};
use objc2_foundation::NSString;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: *const NSString;
}

pub enum Access {
    Granted,
    Denied,
    Restricted,
    Pending(async_channel::Receiver<bool>),
}

pub fn authorization_status() -> Result<isize, String> {
    let device = AnyClass::get(c"AVCaptureDevice")
        .ok_or_else(|| "AVFoundation could not load microphone authorization".to_string())?;
    // SAFETY: AVMediaTypeAudio is a non-null NSString constant exported by
    // AVFoundation, which is linked into this executable.
    let media_type = unsafe { &*AVMediaTypeAudio };
    // SAFETY: This class method accepts AVMediaType and returns NSInteger.
    let status: isize = unsafe { msg_send![device, authorizationStatusForMediaType: media_type] };
    Ok(status)
}

pub fn request_access() -> Result<Access, String> {
    let device = AnyClass::get(c"AVCaptureDevice")
        .ok_or_else(|| "AVFoundation could not load microphone authorization".to_string())?;
    // SAFETY: AVMediaTypeAudio is a non-null NSString constant exported by AVFoundation.
    let media_type = unsafe { &*AVMediaTypeAudio };
    let status = authorization_status()?;
    match status {
        3 => return Ok(Access::Granted),
        0 => {}
        1 => return Ok(Access::Restricted),
        2 => return Ok(Access::Denied),
        _ => {
            return Err(format!(
                "Unexpected microphone authorization status: {status}"
            ));
        }
    }
    let (sender, receiver) = async_channel::bounded(1);
    let completion = RcBlock::new(move |granted: Bool| {
        let _ = sender.try_send(granted.as_bool());
    });
    // SAFETY: AVFoundation copies the block and invokes it after the person
    // answers the system prompt. The receiver keeps the result off the UI thread.
    unsafe {
        let _: () = msg_send![device, requestAccessForMediaType: media_type, completionHandler: &*completion];
    }
    Ok(Access::Pending(receiver))
}
