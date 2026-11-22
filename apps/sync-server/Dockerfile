FROM rust:1.85-slim AS builder
WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/sync-server ./apps/sync-server
COPY apps/desktop/src-tauri/Cargo.toml ./apps/desktop/src-tauri/Cargo.toml
RUN mkdir -p apps/desktop/src-tauri/src && \
    echo "fn main() {}" > apps/desktop/src-tauri/src/main.rs && \
    echo "fn main() {}" > apps/desktop/src-tauri/src/cli.rs && \
    cargo build --release -p fluxcast-sync-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /workspace/target/release/fluxcast-sync-server /usr/local/bin/fluxcast-sync-server
ENV FLUXCAST_BIND=0.0.0.0:8787
EXPOSE 8787
CMD ["fluxcast-sync-server"]
