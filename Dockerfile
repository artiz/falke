# === Build Stage ===
FROM rust:1.91-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies by building a dummy binary first
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -f target/release/falke target/release/deps/falke-*

# Build the actual application
COPY src/ src/
RUN touch src/main.rs && cargo build --release

# === Runtime Stage ===
FROM rust:1.91-slim

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/falke /usr/local/bin/falke

# Run as non-root user
RUN useradd -r -s /bin/false falke
USER falke

ENV RUST_LOG=falke=info

ENTRYPOINT ["falke"]
