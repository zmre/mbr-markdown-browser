// UniFFI build script: generates the Rust scaffolding from the .udl interface
// definition. Apple targets only — the Swift shells are the only consumers, and
// the `uniffi` build-dependency is declared for Apple platforms alone, so on
// Linux/Windows this crate builds empty (see src/lib.rs).

fn main() {
    // The `cfg` keeps the reference to `uniffi` out of build scripts compiled
    // on hosts that do not have the build-dependency; the env check skips the
    // generation for a non-Apple target even on an Apple host.
    #[cfg(target_vendor = "apple")]
    if std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        uniffi::generate_scaffolding("src/mbr.udl").expect("Failed to generate UniFFI scaffolding");
    }
}
