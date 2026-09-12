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


FROM debian:bookworm-slim AS ci

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system kizunalink \
    && adduser --system --ingroup kizunalink kizunalink

WORKDIR /app

ARG TARGETARCH
COPY bin/linux/${TARGETARCH}/kizunalink /app/kizunalink
RUN chmod +x /app/kizunalink && chown kizunalink:kizunalink /app/kizunalink

USER kizunalink
EXPOSE 2333
ENV RUST_LOG=info
ENTRYPOINT ["/app/kizunalink"]


FROM debian:bookworm-slim AS local

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system kizunalink \
    && adduser --system --ingroup kizunalink kizunalink

WORKDIR /app

COPY --from=builder /build/target/release/kizuna-server /app/kizunalink
RUN chmod +x /app/kizunalink && chown kizunalink:kizunalink /app/kizunalink

USER kizunalink
EXPOSE 2333
ENV RUST_LOG=info
ENTRYPOINT ["/app/kizunalink"]