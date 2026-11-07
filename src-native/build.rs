use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=resource.rc");
    println!("cargo:rerun-if-changed=../assets/quotadock.ico");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") { return; }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("quotadock-resource.o");
    let status = Command::new("windres")
        .args(["--input", "resource.rc", "--output"])
        .arg(&out)
        .status()
        .expect("windres is required for the Windows icon");
    assert!(status.success(), "windres failed");
    println!("cargo:rustc-link-arg={}", out.display());
}
