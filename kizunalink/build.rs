// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

//! Build script: provides `libopus` for the raw FFI in `kizuna-voice/opus/ffi.rs`.
//!
//! Resolution order (mirrors the strategy of the retired `audiopus_sys` crate,
//! which this build script replaces — see RUSTSEC-2026-0150):
//!
//! 1. `LIBOPUS_LIB_DIR` / `OPUS_LIB_DIR` — link against a user-provided directory.
//! 2. `pkg-config` probe on non-Windows targets (system `libopus-dev` / `brew install opus`).
//! 3. Vendored CMake build of opus in `native/opus` — static, self-contained. This
//!    is the path used on Windows and in containerized/musl builds without system opus.
//!
//! Static vs dynamic preference: `LIBOPUS_STATIC` / `OPUS_STATIC` force static;
//! Windows, macOS, and musl default to static, other Unix targets to dynamic.

use std::{env, path::Path};

fn main() {
    for var in [
        "LIBOPUS_STATIC",
        "OPUS_STATIC",
        "LIBOPUS_LIB_DIR",
        "OPUS_LIB_DIR",
        "LIBOPUS_NO_PKG",
        "OPUS_NO_PKG",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let is_static = static_pref(&target_os, &target_env);

    // 1. Explicit user-provided library directory wins over everything else.
    if let Some(dir) = env::var_os("LIBOPUS_LIB_DIR").or_else(|| env::var_os("OPUS_LIB_DIR")) {
        let dir = dir.to_string_lossy().to_string();
        println!("cargo:info=Linking opus from user-provided dir: {dir}");
        link(kind_word(is_static), &dir);
        return;
    }

    // 2. pkg-config probe (system opus) on non-Windows targets.
    if target_os != "windows"
        && env::var_os("LIBOPUS_NO_PKG").is_none()
        && env::var_os("OPUS_NO_PKG").is_none()
        && pkg_config::Config::new()
            .statik(is_static)
            .probe("opus")
            .is_ok()
    {
        println!("cargo:info=Found opus via pkg-config.");
        return;
    }

    // 3. Vendored static CMake build (Windows, containers, musl, missing system opus).
    println!("cargo:info=Building vendored opus (native/opus) via CMake.");
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let opus_src = Path::new(&manifest_dir).join("native").join("opus");
    let build_dir = cmake::Config::new(opus_src).build();
    link("static", &build_dir.join("lib").display().to_string());
}

/// Returns the `cargo:rustc-link-lib` kind word for the linking preference.
fn kind_word(is_static: bool) -> &'static str {
    if is_static { "static" } else { "dylib" }
}

/// Whether opus should be linked statically, honoring `LIBOPUS_STATIC`/`OPUS_STATIC`.
fn static_pref(target_os: &str, target_env: &str) -> bool {
    if env::var_os("LIBOPUS_STATIC").is_some() || env::var_os("OPUS_STATIC").is_some() {
        return true;
    }
    target_os == "windows" || target_os == "macos" || target_env == "musl"
}

/// Emits the rustc link directives for `opus` searched in `search_dir`.
fn link(kind: &str, search_dir: &str) {
    println!("cargo:rustc-link-lib={kind}=opus");
    println!("cargo:rustc-link-search=native={search_dir}");
}
