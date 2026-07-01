pub struct SearchConfig {
    pub use_regex: bool,
    pub use_case_sensitivity: bool,
    /// Glob patterns of files to include (VSCode-style "files to include").
    /// When non-empty, only matching files are searched.
    pub includes: Vec<String>,
    /// Glob patterns of files to exclude (VSCode-style "files to exclude").
    /// Applied after includes, so an exclude wins over an include.
    pub excludes: Vec<String>,
}

#[cfg_attr(not(target_family = "wasm"), path = "model.rs")]
#[cfg_attr(target_family = "wasm", path = "model_wasm.rs")]
pub mod model;
pub mod view;
