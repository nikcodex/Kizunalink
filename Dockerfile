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
    AUDIOPUS_STATIC=1 \
    CMAKE_POLICY_VERSION_MINIMUM=3.5 \
    CARGO_TERM_COLOR=always

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
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system kizunalink \
    && adduser --system --ingroup kizunalink kizunalink

WORKDIR /app
USER kizunalink
EXPOSE 2333
ENV RUST_LOG=info
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
