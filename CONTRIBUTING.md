# Contributing to KizunaLink

Thank you for your interest in contributing to KizunaLink!

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/your-username/Kizunalink.git`
3. Install [Rust](https://rustup.rs/) (stable channel, 1.85+)
4. Install system dependencies:
   - **Ubuntu/Debian**: `sudo apt-get install -y libopus-dev cmake pkg-config libclang-dev clang`
   - **macOS**: `brew install opus cmake pkg-config`
5. Build: `cargo build`
6. Run tests: `cargo test`

## Development Workflow

1. Create a feature branch from `main`
2. Make your changes
3. Ensure all checks pass:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --all-targets
   ```
4. Submit a Pull Request

## Code Style

- Follow the existing code conventions
- Use `cargo fmt` (configuration in `rustfmt.toml`)
- Fix all `clippy` warnings before submitting
- Add doc comments (`///`) for public APIs
- Use `tracing` for logging (not `println!`)

## Adding a New Source Plugin

1. Create a new directory under `kizunalink/kizuna-voice/media/sources/your_source/`
2. Implement the `SourcePlugin` trait from `media::sources::plugin`
3. Register it in `media/sources/manager/registration.rs`
4. Add configuration in `config/sources/your_source.rs`
5. Add an example config section in `config.example.toml`

## Reporting Issues

Use the GitHub issue templates for bug reports and feature requests.
