// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

// N05: library code must not print to the console — diagnostics go through `tracing`.
// The two intentional exceptions (the boot banner and the interactive YouTube OAuth flow,
// which runs before/around logger setup) opt out locally with a documented allow.
#![cfg_attr(
    not(test),
    deny(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)
)]

pub mod common;
pub mod config;


pub mod engine;
pub mod opus;
pub mod discord;
pub mod media;
pub mod lavalink;
