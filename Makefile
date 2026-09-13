# KizunaLink development shortcuts.
# Usage: make <target>

.PHONY: fmt fmt-check check clippy test build run deny taplo prepare

# Format the entire workspace.
fmt:
	cargo fmt --all

# Verify formatting without modifying files (CI gate).
fmt-check:
	cargo fmt --all -- --check

# Type-check the whole workspace, all targets.
check:
	cargo check --workspace --all-targets

# Run clippy with warnings as errors.
clippy:
	cargo clippy --workspace --all-targets -- -D warnings

# Run all tests.
test:
	cargo test --workspace --all-targets

# Build release binaries.
build:
	cargo build --release --workspace

# Run the server locally (release profile).
run:
	cargo run -p kizuna-server --release

# Check dependencies against RustSec advisories (requires cargo-deny).
deny:
	cargo deny check advisories

# Format all TOML files (requires taplo).
taplo:
	taplo format

# Everything a PR should pass before pushing.
prepare: fmt-check clippy test
