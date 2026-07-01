use std::ffi::CString;

use pathfinder_color::ColorU;

#[cfg(not(target_family = "wasm"))]
mod mcp;
#[cfg(not(target_family = "wasm"))]
pub(crate) use mcp::ComputerControlMcpBridge;

#[cfg(target_family = "wasm")]
mod mcp {
    pub(crate) fn activate_agent_session(_session_label: &str) {}
    pub(crate) fn deactivate_agent_session(_session_label: Option<&str>) {}
    pub(crate) fn latest_agent_status() -> String {
        "Latest: unavailable".to_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComputerControlChrome {
    pub panel_color: ColorU,
    pub text_color: ColorU,
    pub muted_text_color: ColorU,
    pub glow_color: ColorU,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionGrantState {
    Granted,
    Missing,
    RestartRequired,
    DeniedOrUnknown,
}

impl PermissionGrantState {
    fn is_blocking(self) -> bool {
        !matches!(self, Self::Granted)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComputerControlPermissions {
    pub screen_recording: PermissionGrantState,
    pub accessibility: PermissionGrantState,
}

impl ComputerControlPermissions {
    fn has_blocker(self) -> bool {
        self.screen_recording.is_blocking() || self.accessibility.is_blocking()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputerControlState {
    Stopped,
    Starting,
    Blocked(ComputerControlPermissions),
    Active,
    Stopping,
    Failed(String),
}

impl ComputerControlState {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Starting | Self::Active | Self::Stopping)
    }

    pub fn needs_poll(&self) -> bool {
        matches!(self, Self::Blocked(_) | Self::Active)
    }
}

pub struct ComputerControlCoordinator {
    state: ComputerControlState,
    overlay: Option<OverlayHost>,
    permission_panel: Option<PermissionPanelHost>,
    last_session_label: Option<String>,
    last_chrome: Option<ComputerControlChrome>,
    last_status: Option<String>,
    permission_tracker: PermissionGrantTracker,
    generation: u64,
}

impl Default for ComputerControlCoordinator {
    fn default() -> Self {
        Self {
            state: ComputerControlState::Stopped,
            overlay: None,
            permission_panel: None,
            last_session_label: None,
            last_chrome: None,
            last_status: None,
            permission_tracker: PermissionGrantTracker::default(),
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
            ComputerControlState::Starting
                | ComputerControlState::Blocked(_)
                | ComputerControlState::Active
        ) {
            self.update_chrome(session_label, chrome);
            return;
        }

        self.start_fresh(session_label, chrome);
    }

    fn start_fresh(&mut self, session_label: String, chrome: ComputerControlChrome) {
        self.state = ComputerControlState::Starting;
        self.generation = self.generation.wrapping_add(1);
        self.last_session_label = Some(session_label.clone());
        self.last_chrome = Some(chrome);

        let permissions = self.check_permissions(true);
        if permissions.has_blocker() {
            self.show_blocked_permissions(session_label, chrome, permissions);
            return;
        }

        mcp::activate_agent_session(&session_label);
        let status = mcp::latest_agent_status();
        match OverlayHost::new(&session_label, chrome, &status) {
            Ok(overlay) => {
                self.permission_panel = None;
                self.overlay = Some(overlay);
                self.last_status = Some(status);
                self.state = ComputerControlState::Active;
            }
            Err(error) => {
                mcp::deactivate_agent_session(Some(&session_label));
                self.overlay = None;
                self.permission_panel = None;
                self.last_session_label = None;
                self.last_chrome = None;
                self.last_status = None;
                self.state = ComputerControlState::Failed(error);
            }
        }
    }

    pub fn update_chrome(&mut self, session_label: String, chrome: ComputerControlChrome) {
        if !matches!(
            self.state,
            ComputerControlState::Starting
                | ComputerControlState::Blocked(_)
                | ComputerControlState::Active
        ) {
            return;
        }

        if self.poll_native_events() {
            return;
        }

        let status =
            matches!(self.state, ComputerControlState::Active).then(mcp::latest_agent_status);
        let changed = self.last_session_label.as_ref() != Some(&session_label)
            || self.last_chrome != Some(chrome)
            || status
                .as_ref()
                .is_some_and(|status| self.last_status.as_ref() != Some(status));
        if changed {
            match &self.state {
                ComputerControlState::Blocked(permissions) => {
                    if let Some(panel) = self.permission_panel.as_mut() {
                        panel.update(&session_label, *permissions, chrome);
                    }
                }
                ComputerControlState::Starting | ComputerControlState::Active => {
                    if let Some(overlay) = self.overlay.as_mut() {
                        overlay.update(
                            &session_label,
                            chrome,
                            status.as_deref().unwrap_or("Latest: waiting for Claude"),
                        );
                    }
                }
                ComputerControlState::Stopped
                | ComputerControlState::Stopping
                | ComputerControlState::Failed(_) => {}
            }
            self.last_session_label = Some(session_label);
            self.last_chrome = Some(chrome);
            if let Some(status) = status {
                self.last_status = Some(status);
            }
        }
    }

    pub fn stop(&mut self) {
        if matches!(self.state, ComputerControlState::Stopped) {
            return;
        }

        self.state = ComputerControlState::Stopping;
        mcp::deactivate_agent_session(self.last_session_label.as_deref());
        self.overlay.take();
        self.permission_panel.take();
        self.last_session_label = None;
        self.last_chrome = None;
        self.last_status = None;
        self.state = ComputerControlState::Stopped;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn poll_native_stop(&mut self) -> bool {
        self.poll_native_events()
    }

    pub fn poll_native_events(&mut self) -> bool {
        let stopped = self
            .overlay
            .as_ref()
            .is_some_and(OverlayHost::stop_requested);
        if stopped {
            self.stop();
            return true;
        }

        if matches!(self.state, ComputerControlState::Active) {
            self.refresh_agent_status();
        }

        if matches!(self.state, ComputerControlState::Blocked(_)) {
            let dismissed = self
                .permission_panel
                .as_ref()
                .is_some_and(PermissionPanelHost::dismiss_requested);
            if dismissed {
                self.stop();
                return true;
            }

            let retry = self
                .permission_panel
                .as_ref()
                .is_some_and(PermissionPanelHost::take_retry_requested);
            if retry {
                self.retry_blocked_permissions();
                return true;
            }

            if self.refresh_blocked_permissions() {
                return true;
            }
        }

        false
    }

    fn refresh_agent_status(&mut self) {
        let status = mcp::latest_agent_status();
        if self.last_status.as_ref() == Some(&status) {
            return;
        }
        let (Some(session_label), Some(chrome)) =
            (self.last_session_label.clone(), self.last_chrome)
        else {
            return;
        };
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.update(&session_label, chrome, &status);
        }
        self.last_status = Some(status);
    }

    fn check_permissions(&mut self, prompt_missing: bool) -> ComputerControlPermissions {
        self.permission_tracker
            .evaluate(platform::preflight_permissions(prompt_missing))
    }

    fn show_blocked_permissions(
        &mut self,
        session_label: String,
        chrome: ComputerControlChrome,
        permissions: ComputerControlPermissions,
    ) {
        self.overlay = None;
        self.last_status = None;
        self.state = ComputerControlState::Blocked(permissions);
        match PermissionPanelHost::new(&session_label, permissions, chrome) {
            Ok(panel) => {
                self.permission_panel = Some(panel);
            }
            Err(error) => {
                self.permission_panel = None;
                self.state = ComputerControlState::Failed(error);
            }
        }
    }

    fn retry_blocked_permissions(&mut self) {
        let (Some(session_label), Some(chrome)) =
            (self.last_session_label.clone(), self.last_chrome)
        else {
            self.stop();
            return;
        };

        self.permission_panel.take();
        self.start_fresh(session_label, chrome);
    }

    fn refresh_blocked_permissions(&mut self) -> bool {
        let ComputerControlState::Blocked(previous_permissions) = self.state else {
            return false;
        };
        let permissions = self.check_permissions(false);
        if permissions == previous_permissions {
            return false;
        }

        let (Some(session_label), Some(chrome)) =
            (self.last_session_label.clone(), self.last_chrome)
        else {
            self.stop();
            return true;
        };

        if permissions.has_blocker() {
            self.state = ComputerControlState::Blocked(permissions);
            if let Some(panel) = self.permission_panel.as_mut() {
                panel.update(&session_label, permissions, chrome);
            }
            true
        } else {
            self.permission_panel.take();
            self.start_fresh(session_label, chrome);
            true
        }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PermissionGrantTracker {
    screen_recording_seen_missing: bool,
    accessibility_seen_missing: bool,
}

impl PermissionGrantTracker {
    fn evaluate(&mut self, snapshot: PermissionSnapshot) -> ComputerControlPermissions {
        ComputerControlPermissions {
            screen_recording: self.evaluate_one(
                snapshot.screen_recording,
                snapshot.screen_recording_probe,
                PermissionKind::ScreenRecording,
            ),
            accessibility: self.evaluate_one(
                snapshot.accessibility,
                snapshot.accessibility_probe,
                PermissionKind::Accessibility,
            ),
        }
    }

    fn evaluate_one(
        &mut self,
        grant: NativePermissionGrant,
        probe: NativePermissionProbe,
        kind: PermissionKind,
    ) -> PermissionGrantState {
        if !probe.preflight_granted {
            match kind {
                PermissionKind::ScreenRecording => self.screen_recording_seen_missing = true,
                PermissionKind::Accessibility => self.accessibility_seen_missing = true,
            }
        }

        match grant {
            NativePermissionGrant::Granted => {
                let seen_missing = match kind {
                    PermissionKind::ScreenRecording => self.screen_recording_seen_missing,
                    PermissionKind::Accessibility => self.accessibility_seen_missing,
                };
                if seen_missing {
                    PermissionGrantState::RestartRequired
                } else {
                    PermissionGrantState::Granted
                }
            }
            NativePermissionGrant::Missing => PermissionGrantState::Missing,
            NativePermissionGrant::DeniedOrUnknown => PermissionGrantState::DeniedOrUnknown,
        }
    }
}

#[derive(Clone, Copy)]
enum PermissionKind {
    ScreenRecording,
    Accessibility,
}

#[derive(Clone, Copy)]
enum NativePermissionGrant {
    Granted,
    Missing,
    DeniedOrUnknown,
}

#[derive(Clone, Copy)]
struct NativePermissionProbe {
    preflight_granted: bool,
}

#[derive(Clone, Copy)]
struct PermissionSnapshot {
    screen_recording: NativePermissionGrant,
    screen_recording_probe: NativePermissionProbe,
    accessibility: NativePermissionGrant,
    accessibility_probe: NativePermissionProbe,
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

    use super::{
        sanitized_c_string, ComputerControlChrome, ComputerControlPermissions,
        NativePermissionGrant, NativePermissionProbe, PermissionGrantState, PermissionSnapshot,
    };

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

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NativePermissionSnapshot {
        screen_recording_preflight_granted: bool,
        screen_recording_granted: bool,
        screen_recording_probe_supported: bool,
        accessibility_preflight_granted: bool,
        accessibility_granted: bool,
        accessibility_probe_supported: bool,
    }

    impl From<NativePermissionSnapshot> for PermissionSnapshot {
        fn from(snapshot: NativePermissionSnapshot) -> Self {
            Self {
                screen_recording: native_grant(
                    snapshot.screen_recording_granted,
                    snapshot.screen_recording_probe_supported,
                ),
                screen_recording_probe: NativePermissionProbe {
                    preflight_granted: snapshot.screen_recording_preflight_granted,
                },
                accessibility: native_grant(
                    snapshot.accessibility_granted,
                    snapshot.accessibility_probe_supported,
                ),
                accessibility_probe: NativePermissionProbe {
                    preflight_granted: snapshot.accessibility_preflight_granted,
                },
            }
        }
    }

    fn native_grant(granted: bool, supported: bool) -> NativePermissionGrant {
        if granted {
            NativePermissionGrant::Granted
        } else if supported {
            NativePermissionGrant::Missing
        } else {
            NativePermissionGrant::DeniedOrUnknown
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    enum NativePermissionState {
        Granted = 0,
        Missing = 1,
        RestartRequired = 2,
        DeniedOrUnknown = 3,
    }

    impl From<PermissionGrantState> for NativePermissionState {
        fn from(state: PermissionGrantState) -> Self {
            match state {
                PermissionGrantState::Granted => Self::Granted,
                PermissionGrantState::Missing => Self::Missing,
                PermissionGrantState::RestartRequired => Self::RestartRequired,
                PermissionGrantState::DeniedOrUnknown => Self::DeniedOrUnknown,
            }
        }
    }

    extern "C" {
        fn twarp_computer_control_permissions_preflight(
            prompt_missing: bool,
        ) -> NativePermissionSnapshot;
        fn twarp_computer_control_permissions_panel_create(
            session_label: *const std::ffi::c_char,
            screen_recording_state: NativePermissionState,
            accessibility_state: NativePermissionState,
            panel_color: NativeColor,
            text_color: NativeColor,
            muted_text_color: NativeColor,
            accent_color: NativeColor,
            retry_callback: extern "C" fn(*mut c_void),
            retry_context: *mut c_void,
            dismiss_callback: extern "C" fn(*mut c_void),
            dismiss_context: *mut c_void,
        ) -> *mut c_void;
        fn twarp_computer_control_permissions_panel_update(
            host: *mut c_void,
            session_label: *const std::ffi::c_char,
            screen_recording_state: NativePermissionState,
            accessibility_state: NativePermissionState,
            panel_color: NativeColor,
            text_color: NativeColor,
            muted_text_color: NativeColor,
            accent_color: NativeColor,
        );
        fn twarp_computer_control_permissions_panel_close(host: *mut c_void);
        fn twarp_computer_control_overlay_create(
            session_label: *const std::ffi::c_char,
            status_label: *const std::ffi::c_char,
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
            status_label: *const std::ffi::c_char,
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

    extern "C" fn record_retry_request(context: *mut c_void) {
        if context.is_null() {
            return;
        }
        let retry_requested = unsafe { &*(context as *const AtomicBool) };
        retry_requested.store(true, Ordering::SeqCst);
    }

    extern "C" fn record_dismiss_request(context: *mut c_void) {
        if context.is_null() {
            return;
        }
        let dismiss_requested = unsafe { &*(context as *const AtomicBool) };
        dismiss_requested.store(true, Ordering::SeqCst);
    }

    pub fn preflight_permissions(prompt_missing: bool) -> PermissionSnapshot {
        unsafe { twarp_computer_control_permissions_preflight(prompt_missing).into() }
    }

    pub struct OverlayHost {
        host: NonNull<c_void>,
        stop_requested: Arc<AtomicBool>,
        stop_context: *const AtomicBool,
    }

    impl OverlayHost {
        pub fn new(
            session_label: &str,
            chrome: ComputerControlChrome,
            status: &str,
        ) -> Result<Self, String> {
            let session_label = sanitized_c_string(session_label);
            let status = sanitized_c_string(status);
            let stop_requested = Arc::new(AtomicBool::new(false));
            let stop_context = Arc::into_raw(stop_requested.clone());
            let host = unsafe {
                twarp_computer_control_overlay_create(
                    session_label.as_ptr(),
                    status.as_ptr(),
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

        pub fn update(&mut self, session_label: &str, chrome: ComputerControlChrome, status: &str) {
            let session_label = sanitized_c_string(session_label);
            let status = sanitized_c_string(status);
            unsafe {
                twarp_computer_control_overlay_update(
                    self.host.as_ptr(),
                    session_label.as_ptr(),
                    status.as_ptr(),
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

    pub struct PermissionPanelHost {
        host: NonNull<c_void>,
        retry_requested: Arc<AtomicBool>,
        retry_context: *const AtomicBool,
        dismiss_requested: Arc<AtomicBool>,
        dismiss_context: *const AtomicBool,
    }

    impl PermissionPanelHost {
        pub fn new(
            session_label: &str,
            permissions: ComputerControlPermissions,
            chrome: ComputerControlChrome,
        ) -> Result<Self, String> {
            let session_label = sanitized_c_string(session_label);
            let retry_requested = Arc::new(AtomicBool::new(false));
            let retry_context = Arc::into_raw(retry_requested.clone());
            let dismiss_requested = Arc::new(AtomicBool::new(false));
            let dismiss_context = Arc::into_raw(dismiss_requested.clone());
            let host = unsafe {
                twarp_computer_control_permissions_panel_create(
                    session_label.as_ptr(),
                    permissions.screen_recording.into(),
                    permissions.accessibility.into(),
                    chrome.panel_color.into(),
                    chrome.text_color.into(),
                    chrome.muted_text_color.into(),
                    chrome.glow_color.into(),
                    record_retry_request,
                    retry_context as *mut c_void,
                    record_dismiss_request,
                    dismiss_context as *mut c_void,
                )
            };
            let Some(host) = NonNull::new(host) else {
                unsafe {
                    drop(Arc::from_raw(retry_context));
                    drop(Arc::from_raw(dismiss_context));
                }
                return Err("failed to create computer-control permissions panel".to_owned());
            };
            Ok(Self {
                host,
                retry_requested,
                retry_context,
                dismiss_requested,
                dismiss_context,
            })
        }

        pub fn update(
            &mut self,
            session_label: &str,
            permissions: ComputerControlPermissions,
            chrome: ComputerControlChrome,
        ) {
            let session_label = sanitized_c_string(session_label);
            unsafe {
                twarp_computer_control_permissions_panel_update(
                    self.host.as_ptr(),
                    session_label.as_ptr(),
                    permissions.screen_recording.into(),
                    permissions.accessibility.into(),
                    chrome.panel_color.into(),
                    chrome.text_color.into(),
                    chrome.muted_text_color.into(),
                    chrome.glow_color.into(),
                );
            }
        }

        pub fn take_retry_requested(&self) -> bool {
            self.retry_requested.swap(false, Ordering::SeqCst)
        }

        pub fn dismiss_requested(&self) -> bool {
            self.dismiss_requested.load(Ordering::SeqCst)
        }
    }

    impl Drop for PermissionPanelHost {
        fn drop(&mut self) {
            unsafe {
                twarp_computer_control_permissions_panel_close(self.host.as_ptr());
                drop(Arc::from_raw(self.retry_context));
                drop(Arc::from_raw(self.dismiss_context));
            }
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
    use super::{
        ComputerControlChrome, ComputerControlPermissions, NativePermissionGrant,
        NativePermissionProbe, PermissionSnapshot,
    };

    pub fn preflight_permissions(_prompt_missing: bool) -> PermissionSnapshot {
        PermissionSnapshot {
            screen_recording: NativePermissionGrant::DeniedOrUnknown,
            screen_recording_probe: NativePermissionProbe {
                preflight_granted: false,
            },
            accessibility: NativePermissionGrant::DeniedOrUnknown,
            accessibility_probe: NativePermissionProbe {
                preflight_granted: false,
            },
        }
    }

    pub struct OverlayHost;

    impl OverlayHost {
        pub fn new(
            _session_label: &str,
            _chrome: ComputerControlChrome,
            _status: &str,
        ) -> Result<Self, String> {
            Err("computer control overlay is only available on macOS".to_owned())
        }

        pub fn update(
            &mut self,
            _session_label: &str,
            _chrome: ComputerControlChrome,
            _status: &str,
        ) {
        }

        pub fn stop_requested(&self) -> bool {
            false
        }
    }

    pub struct PermissionPanelHost;

    impl PermissionPanelHost {
        pub fn new(
            _session_label: &str,
            _permissions: ComputerControlPermissions,
            _chrome: ComputerControlChrome,
        ) -> Result<Self, String> {
            Err("computer control permissions are only available on macOS".to_owned())
        }

        pub fn update(
            &mut self,
            _session_label: &str,
            _permissions: ComputerControlPermissions,
            _chrome: ComputerControlChrome,
        ) {
        }

        pub fn take_retry_requested(&self) -> bool {
            false
        }

        pub fn dismiss_requested(&self) -> bool {
            false
        }
    }
}

use platform::{OverlayHost, PermissionPanelHost};

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(
        screen_preflight: bool,
        screen_grant: NativePermissionGrant,
        accessibility_preflight: bool,
        accessibility_grant: NativePermissionGrant,
    ) -> PermissionSnapshot {
        PermissionSnapshot {
            screen_recording: screen_grant,
            screen_recording_probe: NativePermissionProbe {
                preflight_granted: screen_preflight,
            },
            accessibility: accessibility_grant,
            accessibility_probe: NativePermissionProbe {
                preflight_granted: accessibility_preflight,
            },
        }
    }

    #[test]
    fn missing_permission_that_becomes_granted_is_restart_required() {
        let mut tracker = PermissionGrantTracker::default();
        let missing = tracker.evaluate(snapshot(
            false,
            NativePermissionGrant::Missing,
            true,
            NativePermissionGrant::Granted,
        ));
        assert_eq!(missing.screen_recording, PermissionGrantState::Missing);
        assert_eq!(missing.accessibility, PermissionGrantState::Granted);

        let granted_after_request = tracker.evaluate(snapshot(
            true,
            NativePermissionGrant::Granted,
            true,
            NativePermissionGrant::Granted,
        ));
        assert_eq!(
            granted_after_request.screen_recording,
            PermissionGrantState::RestartRequired
        );
        assert_eq!(
            granted_after_request.accessibility,
            PermissionGrantState::Granted
        );
    }

    #[test]
    fn permissions_granted_before_start_are_not_restart_required() {
        let mut tracker = PermissionGrantTracker::default();
        let permissions = tracker.evaluate(snapshot(
            true,
            NativePermissionGrant::Granted,
            true,
            NativePermissionGrant::Granted,
        ));
        assert_eq!(permissions.screen_recording, PermissionGrantState::Granted);
        assert_eq!(permissions.accessibility, PermissionGrantState::Granted);
        assert!(!permissions.has_blocker());
    }
}
