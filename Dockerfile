# ============================================
#  ⛩️ KizunaLink — Production Dockerfile
#  Multi-stage build for minimal image size
# ============================================

# Stage 1: Build
FROM rust:1.80-bookworm AS builder

WORKDIR /app

# Cache dependencies first (changes less often)
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -rf src

# Copy source and build
COPY src/ src/
COPY plugins/ plugins/
RUN touch src/main.rs && cargo build --release

# Stage 2: Runtime (minimal image)
FROM debian:bookworm-slim AS runtime

# Install minimal runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        fonts-liberation \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/kizunalink ./kizunalink

# Copy config and plugins
COPY config.toml ./config.toml
COPY plugins/ ./plugins/

# Create non-root user for security
RUN useradd -r -s /bin/false -m -d /app kizuna && \
    chown -R kizuna:kizuna /app
USER kizuna

# Expose default port
EXPOSE 2333

# Health check
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
    CMD curl -sf http://localhost:2333/ || exit 1

# Start KizunaLink
CMD ["./kizunalink"]
