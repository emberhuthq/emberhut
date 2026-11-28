//! On Windows (MSVC), the server's main thread gets an 8 MiB stack, as on Linux: Windows gives
//! it 1 MiB, which a debug build's start-up overflows. Nothing is done for other systems.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
}
