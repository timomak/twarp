fn main() -> Result<(), std::env::VarError> {
    let target_family = std::env::var("CARGO_CFG_TARGET_FAMILY")?;

    if target_family != "wasm" {
        println!("cargo:rustc-cfg=feature=\"local_fs\"");
    }

    // `embed_migrations!("migrations")` bakes the migration set into the binary at
    // COMPILE time. Cargo does not treat that directory as an input to this crate,
    // so adding a new migration would not, on its own, recompile `persistence` —
    // the binary would keep shipping a stale migration set and `run_pending_migrations`
    // would silently skip the new tables at runtime. (This bit us once already: a
    // binary that referenced `claude_code_panes` in every snapshot save shipped
    // without the migration that creates it, so every save aborted on "no such
    // table" and all session persistence broke.) Track the directory so any change
    // to the migration set forces a rebuild and re-embed.
    println!("cargo:rerun-if-changed=migrations");

    Ok(())
}
