fn main() {
    println!("cargo:rerun-if-changed=memory.x");

    // Only the ARM firmware build wants the Cortex-M linker scripts. Host test
    // builds (cargo test on the dev machine) must not get them, and there is no
    // bin target to apply them to when running tests against the library.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("thumbv") {
        println!("cargo:rustc-link-arg-bins=--nmagic");
        println!("cargo:rustc-link-arg-bins=-Tlink.x");
        println!("cargo:rustc-link-arg-bins=-Tdefmt.x");
    }
}
