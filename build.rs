use std::{env, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    for asset in ["wiki.rc", "Wiki the boy (1).ico", "Wiki the boy.ico"] {
        println!("cargo:rerun-if-changed=assets/{asset}");
    }

    let compiler = find_msvc_tools::find_windows_sdk(env::consts::ARCH)
        .and_then(|sdk| {
            sdk.path()
                .map(|dir| dir.join("rc.exe"))
                .find(|rc| rc.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("rc.exe"));
    let resource =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo did not set OUT_DIR")).join("wiki.res");
    let status = Command::new(compiler)
        .current_dir("assets")
        .args(["/nologo", "/fo"])
        .arg(&resource)
        .arg("wiki.rc")
        .status()
        .expect("Could not start rc.exe. Install the Windows SDK with Visual Studio Build Tools.");
    assert!(status.success(), "Could not compile the Windows icons");
    println!("cargo:rustc-link-arg-bin=wiki={}", resource.display());
}
