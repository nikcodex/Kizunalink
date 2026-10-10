# syntax=docker/dockerfile:1
#
# KizunaLink — single multi-stage Dockerfile.
#
# Stages:
#   builder       — compiles the release binary from source.
#   runtime-base  — shared, hardened runtime (non-root user, slim Debian).
#   ci            — runtime image from a prebuilt binary (used by release CI).
#   local         — DEFAULT target: runtime image built from source in this repo.
#
# Usage:
#   docker build -t kizunalink .            # builds from source (default target)
#   docker build --target ci -t kizunalink . # assembles from bin/linux/<arch>/kizunalink

# ---------------------------------------------------------------------------
# Stage 1: builder — compile from source.
# ---------------------------------------------------------------------------
FROM rust:1.93-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    pkg-config \
    libclang-dev \
    clang \
    build-essential \
    perl \
    git \
    && rm -rf /var/lib/apt/lists/*

ENV LIBOPUS_STATIC=1 \
    OPUS_STATIC=1 \
    CARGO_TERM_COLOR=always

# Optional build metadata, baked in via option_env! and shown in the startup
# banner and /v4/info. Pass with `--build-arg` in CI; unset locally.
ARG BUILD_TIME \
    BUILD_TIME_HUMAN \
    GIT_BRANCH \
    GIT_COMMIT \
    GIT_COMMIT_SHORT \
    GIT_COMMIT_TIME \
    GIT_COMMIT_TIME_HUMAN
ENV BUILD_TIME=${BUILD_TIME} \
    BUILD_TIME_HUMAN=${BUILD_TIME_HUMAN} \
    GIT_BRANCH=${GIT_BRANCH} \
    GIT_COMMIT=${GIT_COMMIT} \
    GIT_COMMIT_SHORT=${GIT_COMMIT_SHORT} \
    GIT_COMMIT_TIME=${GIT_COMMIT_TIME} \
    GIT_COMMIT_TIME_HUMAN=${GIT_COMMIT_TIME_HUMAN}

WORKDIR /build
COPY . .
RUN cargo build --release --locked

# ---------------------------------------------------------------------------
# Stage 2: runtime-base — shared, non-root runtime.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime-base

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system kizunalink \
    && adduser --system --ingroup kizunalink kizunalink

WORKDIR /app
USER kizunalink
EXPOSE 2333
ENV RUST_LOG=info
# `/health` is unauthenticated and cheap, so it is safe to poll from orchestrators.
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:2333/health || exit 1
ENTRYPOINT ["/app/kizunalink"]

# ---------------------------------------------------------------------------
# Stage 3: ci — assemble from a prebuilt binary.
# ---------------------------------------------------------------------------
FROM runtime-base AS ci
ARG TARGETARCH
COPY --chown=kizunalink:kizunalink bin/linux/${TARGETARCH}/kizunalink /app/kizunalink
RUN chmod +x /app/kizunalink

# ---------------------------------------------------------------------------
# Stage 4 (DEFAULT): local — run the binary built in stage 1.
# ---------------------------------------------------------------------------
FROM runtime-base AS local
COPY --from=builder --chown=kizunalink:kizunalink /build/target/release/kizuna-server /app/kizunalink
RUN chmod +x /app/kizunalink
