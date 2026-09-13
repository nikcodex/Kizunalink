// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

use crate::common::utils::{BOLD, CYAN, DIM, ORANGE, RESET, YELLOW, colors_enabled, strip_ansi_escapes};

/// Prints one banner line: console *and* log file via `log_println!`, with ANSI codes
/// stripped when colors are disabled (N07). The allow keeps N05's `print_stdout` gate from
/// firing inside the macro expansion — this module is the documented, deliberate exception.
#[allow(clippy::print_stdout)]
fn emit(line: String) {
    if colors_enabled() {
        crate::log_println!("{line}");
    } else {
        crate::log_println!("{}", strip_ansi_escapes(&line));
    }
}

macro_rules! env_or {
    ($key:literal, $default:literal) => {
        option_env!($key).unwrap_or($default)
    };
}

pub struct BannerInfo {
    pub version: &'static str,
    pub build_time: &'static str,
    pub branch: &'static str,
    pub commit: &'static str,
    pub commit_short: &'static str,
    pub commit_time: &'static str,
    pub rust_version: &'static str,
    pub dirty: bool,
    pub profile: &'static str,
}

impl Default for BannerInfo {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            build_time: env_or!("BUILD_TIME_HUMAN", "unknown"),
            branch: env_or!("GIT_BRANCH", "unknown"),
            commit: env_or!("GIT_COMMIT", "unknown"),
            commit_short: env_or!("GIT_COMMIT_SHORT", "unknown"),
            commit_time: env_or!("GIT_COMMIT_TIME_HUMAN", "unknown"),
            rust_version: env_or!("RUST_VERSION", "unknown"),
            dirty: matches!(option_env!("GIT_DIRTY"), Some("true")),
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
        }
    }
}

pub fn print_banner(info: &BannerInfo) {
    const BANNER_ART: &str = r#"
    __ __ _                           __    _       __   
   / //_/(_)____ __  __ ____   ____ _ / /   (_)____ / /__ 
  / ,<  / //_  // / / // __ \ / __ `// /   / // __ \/ //_/
 / /| |/ /  / /_ / /_/ // / / // /_/ // /___/ // / / / ,<  
/_/ |_/_/  /___/ \__,_//_/ /_/ \__,_//_____/_//_/ /_/_/|_| 
"#;

    emit(format!("{ORANGE}{BANNER_ART}{RESET}"));
    emit(format!("{DIM}========================================{RESET}\n"));

    print_row("Version", info.version, CYAN);
    print_row("Build time", info.build_time, RESET);
    print_row("Branch", info.branch, RESET);

    let commit_display = if info.dirty {
        format!("{}{YELLOW} (dirty){RESET}", info.commit_short)
    } else {
        info.commit_short.to_owned()
    };
    print_row("Commit", &commit_display, RESET);
    print_row("Commit time", info.commit_time, RESET);
    print_row("Rust", info.rust_version, RESET);
    print_row("Profile", info.profile, YELLOW);

    emit(format!(
        "\n{DIM}  No active profile set, falling back to 1 default profile: \
         \"{BOLD}default{RESET}{DIM}\"{RESET}\n"
    ));
}

fn print_row(label: &str, value: impl AsRef<str>, color: &str) {
    emit(format!("  {BOLD}{label:<14}{RESET}{color}{}{RESET}", value.as_ref()));
}
