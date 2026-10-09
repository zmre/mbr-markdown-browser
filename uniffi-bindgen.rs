// UniFFI binding generator binary
// Run with (after `cargo build --release -p mbr-ffi --lib`):
//   cargo run --features ffi --bin uniffi-bindgen -- generate \
//     --library target/release/libmbr_ffi.a --language swift --out-dir apple/quicklook/Generated

fn main() {
    uniffi::uniffi_bindgen_main();
}
