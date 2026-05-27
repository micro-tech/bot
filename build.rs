fn main() {
    // Inject a build timestamp so the installer can print it at runtime.
    // This makes it easy to confirm a fresh binary is actually being used.
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", timestamp);

    // Re-run whenever the installer source or either embedded config changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/bin/installer.rs");
    println!("cargo:rerun-if-changed=config.toml");
    println!("cargo:rerun-if-changed=system_manifest.md");
}
