// SPDX-License-Identifier: MPL-2.0
// Copyright (C) 2026 149segolte <me@149segolte.dev>

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo sets the target OS");
    // ash's linked feature emits the platform-specific Vulkan link directive.
    // Here we only need to locate the library for the linker.
    if target_os != "windows" && pkg_config::probe_library("vulkan").is_ok() {
        return;
    }
    if let Some(sdk_dir) = std::env::var_os("VULKAN_SDK") {
        let directory = if target_os == "windows" {
            if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86") {
                "Lib32"
            } else {
                "Lib"
            }
        } else {
            "lib"
        };
        let path = std::path::PathBuf::from(sdk_dir).join(directory);
        println!("cargo:rustc-link-search=native={}", path.display());
    } else if target_os != "windows" {
        println!(
            "cargo:warning=Vulkan was not found via pkg-config and VULKAN_SDK is unset; relying on the linker's default search paths"
        );
    }
}
