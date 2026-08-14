use std::{env, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    let developer_dir = env::var_os("DEVELOPER_DIR").map(PathBuf::from).or_else(|| {
        let output = Command::new("xcode-select").arg("-p").output().ok()?;
        output
            .status
            .success()
            .then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string()))
    });
    if let Some(developer_dir) = developer_dir {
        let toolchain = developer_dir.join("Toolchains/XcodeDefault.xctoolchain/usr/lib");
        for relative in ["swift-5.5/macosx", "swift/macosx"] {
            let path = toolchain.join(relative);
            if path.is_dir() {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{}", path.display());
            }
        }
    }
}
