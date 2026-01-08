// UniFFI binding generator binary
// Run with: cargo run --bin uniffi-bindgen -- generate --library target/release/libmbr.a --language swift --out-dir quicklook/Generated

fn main() {
    uniffi::uniffi_bindgen_main();
}
