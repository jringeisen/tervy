//! Pinned GPUI has no badge API; AppKit owns this small main-thread adapter.
pub fn set(count: usize) {
    let Some(main) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(main);
    let text = (count > 0).then(|| objc2_foundation::NSString::from_str(&count.to_string()));
    app.dockTile().setBadgeLabel(text.as_deref());
}
