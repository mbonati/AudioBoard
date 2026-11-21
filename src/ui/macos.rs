//! macOS: the native window, for what winit doesn't expose.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSFloatingWindowLevel, NSView, NSWindow};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The app's NSWindow.
pub struct NativeWindow(Retained<NSWindow>);

impl NativeWindow {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let handle = cc.window_handle().ok()?;
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return None;
        };
        // SAFETY: an AppKit handle's `ns_view` is the live NSView of the
        // window being created, and app creation runs on the main thread.
        let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        view.window().map(Self)
    }

    /// Whether the window server may drag the window. The toolbar sits in
    /// the (transparent) title bar, where a drag otherwise moves the window
    /// before the app sees it: controls that are dragged turn this off
    /// while the pointer is on them.
    pub fn set_movable(&self, movable: bool) {
        if self.0.isMovable() != movable {
            self.0.setMovable(movable);
        }
    }
}

/// The app's windows as they are now, to tell which one a call opens.
pub fn windows() -> Vec<Retained<NSWindow>> {
    MainThreadMarker::new().map_or_else(Vec::new, |mtm| {
        NSApplication::sharedApplication(mtm).windows().to_vec()
    })
}

/// Floats the windows opened since `before` (plugin editors) above the
/// board: they stay in front of it while AudioBoard is the active app, and
/// hide while another app is, as plugin windows do in DAWs.
pub fn float_new_windows(before: &[Retained<NSWindow>]) {
    for w in windows() {
        if before.iter().all(|b| !std::ptr::eq(&**b, &*w)) {
            w.setLevel(NSFloatingWindowLevel);
            w.setHidesOnDeactivate(true);
        }
    }
}
