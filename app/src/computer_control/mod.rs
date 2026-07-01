use std::ffi::CString;

use pathfinder_color::ColorU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComputerControlChrome {
    pub panel_color: ColorU,
    pub text_color: ColorU,
    pub muted_text_color: ColorU,
    pub glow_color: ColorU,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputerControlState {
    Stopped,
    Starting,
    Active,
    Stopping,
    Failed(String),
}

impl ComputerControlState {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Starting | Self::Active | Self::Stopping)
    }
}

pub struct ComputerControlCoordinator {
    state: ComputerControlState,
    overlay: Option<OverlayHost>,
    last_session_label: Option<String>,
    last_chrome: Option<ComputerControlChrome>,
    generation: u64,
}

impl Default for ComputerControlCoordinator {
    fn default() -> Self {
        Self {
            state: ComputerControlState::Stopped,
            overlay: None,
            last_session_label: None,
            last_chrome: None,
            generation: 0,
        }
    }
}

impl ComputerControlCoordinator {
    pub fn state(&self) -> &ComputerControlState {
        &self.state
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn start(&mut self, session_label: String, chrome: ComputerControlChrome) {
        if matches!(
            self.state,
            ComputerControlState::Starting | ComputerControlState::Active
        ) {
            self.update_chrome(session_label, chrome);
            return;
        }

        self.state = ComputerControlState::Starting;
        self.generation = self.generation.wrapping_add(1);
        match OverlayHost::new(&session_label, chrome) {
            Ok(overlay) => {
                self.overlay = Some(overlay);
                self.last_session_label = Some(session_label);
                self.last_chrome = Some(chrome);
                self.state = ComputerControlState::Active;
            }
            Err(error) => {
                self.overlay = None;
                self.last_session_label = None;
                self.last_chrome = None;
                self.state = ComputerControlState::Failed(error);
            }
        }
    }

    pub fn update_chrome(&mut self, session_label: String, chrome: ComputerControlChrome) {
        if !matches!(
            self.state,
            ComputerControlState::Starting | ComputerControlState::Active
        ) {
            return;
        }

        if self.poll_native_stop() {
            return;
        }

        let changed = self.last_session_label.as_ref() != Some(&session_label)
            || self.last_chrome != Some(chrome);
        if changed {
            if let Some(overlay) = self.overlay.as_mut() {
                overlay.update(&session_label, chrome);
            }
            self.last_session_label = Some(session_label);
            self.last_chrome = Some(chrome);
        }
    }

    pub fn stop(&mut self) {
        if matches!(self.state, ComputerControlState::Stopped) {
            return;
        }

        self.state = ComputerControlState::Stopping;
        self.overlay.take();
        self.last_session_label = None;
        self.last_chrome = None;
        self.state = ComputerControlState::Stopped;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn poll_native_stop(&mut self) -> bool {
        let stopped = self
            .overlay
            .as_ref()
            .is_some_and(OverlayHost::stop_requested);
        if stopped {
            self.stop();
        }
        stopped
    }
}

impl Drop for ComputerControlCoordinator {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn platform_supported() -> bool {
    cfg!(target_os = "macos")
}

fn sanitized_c_string(value: &str) -> CString {
    CString::new(value.replace('\0', " ")).expect("interior nul bytes were replaced")
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use pathfinder_color::ColorU;

    use super::{sanitized_c_string, ComputerControlChrome};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NativeColor {
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    }

    impl From<ColorU> for NativeColor {
        fn from(color: ColorU) -> Self {
            Self {
                r: color.r,
                g: color.g,
                b: color.b,
                a: color.a,
            }
        }
    }

    extern "C" {
        fn twarp_computer_control_overlay_create(
            session_label: *const std::ffi::c_char,
            panel_color: NativeColor,
            text_color: NativeColor,
            muted_text_color: NativeColor,
            glow_color: NativeColor,
            stop_callback: extern "C" fn(*mut c_void),
            stop_context: *mut c_void,
        ) -> *mut c_void;
        fn twarp_computer_control_overlay_update(
            host: *mut c_void,
            session_label: *const std::ffi::c_char,
            panel_color: NativeColor,
            text_color: NativeColor,
            muted_text_color: NativeColor,
            glow_color: NativeColor,
        );
        fn twarp_computer_control_overlay_close(host: *mut c_void);
    }

    extern "C" fn record_stop_request(context: *mut c_void) {
        if context.is_null() {
            return;
        }
        let stop_requested = unsafe { &*(context as *const AtomicBool) };
        stop_requested.store(true, Ordering::SeqCst);
    }

    pub struct OverlayHost {
        host: NonNull<c_void>,
        stop_requested: Arc<AtomicBool>,
        stop_context: *const AtomicBool,
    }

    impl OverlayHost {
        pub fn new(session_label: &str, chrome: ComputerControlChrome) -> Result<Self, String> {
            let session_label = sanitized_c_string(session_label);
            let stop_requested = Arc::new(AtomicBool::new(false));
            let stop_context = Arc::into_raw(stop_requested.clone());
            let host = unsafe {
                twarp_computer_control_overlay_create(
                    session_label.as_ptr(),
                    chrome.panel_color.into(),
                    chrome.text_color.into(),
                    chrome.muted_text_color.into(),
                    chrome.glow_color.into(),
                    record_stop_request,
                    stop_context as *mut c_void,
                )
            };
            let Some(host) = NonNull::new(host) else {
                unsafe {
                    drop(Arc::from_raw(stop_context));
                }
                return Err("failed to create computer-control overlay windows".to_owned());
            };
            Ok(Self {
                host,
                stop_requested,
                stop_context,
            })
        }

        pub fn update(&mut self, session_label: &str, chrome: ComputerControlChrome) {
            let session_label = sanitized_c_string(session_label);
            unsafe {
                twarp_computer_control_overlay_update(
                    self.host.as_ptr(),
                    session_label.as_ptr(),
                    chrome.panel_color.into(),
                    chrome.text_color.into(),
                    chrome.muted_text_color.into(),
                    chrome.glow_color.into(),
                );
            }
        }

        pub fn stop_requested(&self) -> bool {
            self.stop_requested.load(Ordering::SeqCst)
        }
    }

    impl Drop for OverlayHost {
        fn drop(&mut self) {
            unsafe {
                twarp_computer_control_overlay_close(self.host.as_ptr());
                drop(Arc::from_raw(self.stop_context));
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::ComputerControlChrome;

    pub struct OverlayHost;

    impl OverlayHost {
        pub fn new(_session_label: &str, _chrome: ComputerControlChrome) -> Result<Self, String> {
            Err("computer control overlay is only available on macOS".to_owned())
        }

        pub fn update(&mut self, _session_label: &str, _chrome: ComputerControlChrome) {}

        pub fn stop_requested(&self) -> bool {
            false
        }
    }
}

use platform::OverlayHost;
