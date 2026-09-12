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
