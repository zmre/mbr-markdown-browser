// UniFFI build script for generating Swift bindings
// This generates Rust scaffolding from the .udl interface definition
// Only runs when the "ffi" feature is enabled

fn main() {
    // Only generate UniFFI scaffolding when building with ffi feature
    #[cfg(feature = "ffi")]
    {
        uniffi::generate_scaffolding("src/mbr.udl").expect("Failed to generate UniFFI scaffolding");
    }
}
