# === Build Stage ===
FROM rust:1.88-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies by building them first
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true
RUN rm -rf src

# Build the actual application
COPY src/ src/
RUN cargo build --release

# === Runtime Stage ===
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/falke /usr/local/bin/falke

# Run as non-root user
RUN useradd -r -s /bin/false falke
USER falke

ENV RUST_LOG=falke=info

ENTRYPOINT ["falke"]
