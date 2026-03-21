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
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates libssl3 wget && \
    # Install ONNX Runtime 1.24.4 shared library
    wget -q https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/onnxruntime-linux-x64-1.24.4.tgz && \
    tar xzf onnxruntime-linux-x64-1.24.4.tgz && \
    cp onnxruntime-linux-x64-1.24.4/lib/libonnxruntime.so.1.24.4 /usr/local/lib/ && \
    ldconfig && \
    rm -rf onnxruntime-linux-x64-1.24.4* && \
    apt-get remove -y wget && apt-get autoremove -y && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/falke /usr/local/bin/falke

# ONNX Runtime — point ort (load-dynamic) at the installed library
ENV ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1.24.4

# Run as non-root user
RUN useradd -r -s /bin/false falke
USER falke

ENV RUST_LOG=falke=info

ENTRYPOINT ["falke"]
