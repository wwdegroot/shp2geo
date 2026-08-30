# shapefile-converter
#
# Build:
#   docker build -t shapefile-converter .
# Run:
#   docker run -p 3000:3000 shapefile-converter

ARG TARGETPLATFORM
ARG TARGET_TRIPLE=x86_64-unknown-linux-musl

# --- Stage 1: static musl build -------------------------------------------
FROM rust:1-slim AS builder

ARG TARGET_TRIPLE

RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add "${TARGET_TRIPLE}"

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --target "${TARGET_TRIPLE}" \
    && strip "/app/target/${TARGET_TRIPLE}/release/shapefile-converter"

# --- Stage 2: distroless runtime -------------------------------------------
# Static binary (musl) -> distroless/static. No shell, no package manager.
FROM gcr.io/distroless/static-debian12:nonroot

ARG TARGET_TRIPLE

COPY --from=builder /app/target/${TARGET_TRIPLE}/release/shapefile-converter /shapefile-converter

ENV PORT=3000
EXPOSE 3000

# NOTE: distroless has no shell/curl, so a HEALTHCHECK instruction cannot run
# here. Configure your orchestrator to probe GET /healthz directly
# (e.g. Kubernetes liveness/readiness probes with httpGet).

USER nonroot:nonroot
ENTRYPOINT ["/shapefile-converter"]
