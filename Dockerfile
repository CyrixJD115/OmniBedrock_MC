# syntax=docker/dockerfile:1
# The panel UI is built as a static SPA and embedded into the Rust binary, so
# the runtime image is one executable serving both the web panel and the API.
FROM node:22-bookworm-slim AS ui
WORKDIR /frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN cp vite.config.example.ts vite.config.ts && npm run build

FROM rust:1.90-bookworm AS build
WORKDIR /build
ENV OMNI_SKIP_FRONTEND_BUILD=1
COPY Cargo.toml Cargo.lock ./
COPY backend/omnibedrock-core/Cargo.toml backend/omnibedrock-core/Cargo.toml
COPY backend/omnibedrock-server/Cargo.toml backend/omnibedrock-server/Cargo.toml
COPY backend/omnibedrock-api/Cargo.toml backend/omnibedrock-api/Cargo.toml
# Pre-build the dependency graph against stub sources so later source-only
# changes reuse the cached dependency artifacts.
RUN mkdir -p backend/omnibedrock-core/src backend/omnibedrock-server/src backend/omnibedrock-api/src \
    && echo "" > backend/omnibedrock-core/src/lib.rs \
    && echo "" > backend/omnibedrock-server/src/lib.rs \
    && echo "" > backend/omnibedrock-api/src/lib.rs \
    && echo "fn main() {}" > backend/omnibedrock-api/src/main.rs \
    && cargo build --release --locked --bin omnibedrock \
    && rm -rf backend/omnibedrock-core/src backend/omnibedrock-server/src backend/omnibedrock-api/src target/release/omnibedrock
COPY backend/omnibedrock-core/src backend/omnibedrock-core/src
COPY backend/omnibedrock-server/src backend/omnibedrock-server/src
COPY backend/omnibedrock-api/src backend/omnibedrock-api/src
# openapi.json is include_str!'d from api/router.rs, so it must exist for the
# build even though src/ above is replaced by dummies in the deps stage.
COPY backend/omnibedrock-api/openapi.json backend/omnibedrock-api/openapi.json
# The build script lives at the crate root, outside src/, so it needs its own
# COPY (touching it would create an empty file and fail cargo's auto-detect).
COPY backend/omnibedrock-api/build.rs backend/omnibedrock-api/build.rs
COPY --from=ui /frontend/build frontend/build
RUN touch backend/omnibedrock-core/src/lib.rs backend/omnibedrock-server/src/lib.rs backend/omnibedrock-api/src/lib.rs backend/omnibedrock-api/src/main.rs frontend/build/index.html \
    && cargo build --release --locked --bin omnibedrock

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libstdc++6 curl && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home omni \
    && mkdir -p /data/bedrock /data/backups /data/panel /data/logs && chown -R omni:omni /data
COPY --from=build /build/target/release/omnibedrock /usr/local/bin/omnibedrock
ENV BEDROCK_SERVER_DIR=/data/bedrock BACKUPS_DIR=/data/backups DATA_DIR=/data/panel LOGS_DIR=/data/logs OMNI_PORT=17754
WORKDIR /data
USER omni
EXPOSE 17754 19132/udp 19133/udp
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s CMD curl -fsS http://127.0.0.1:17754/health || exit 1
ENTRYPOINT ["omnibedrock"]

# The panel is Rust. Python is installed here only for the Endstone game launcher.
# Java is included for the optional MCXboxBroadcast standalone integration.
FROM runtime AS endstone
USER root
ARG ENDSTONE_VERSION=0.11.14
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv libcurl4 default-jre-headless \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/endstone \
    && /opt/endstone/bin/pip install --no-cache-dir "endstone==${ENDSTONE_VERSION}" \
    && mkdir -p /data/broadcaster && chown omni:omni /data/broadcaster
ENV ENDSTONE_BIN=/opt/endstone/bin/endstone BROADCASTER_DIR=/data/broadcaster
USER omni
