# Build stage
FROM rust:1-bookworm AS builder

WORKDIR /src

# Cache dependency compilation separately from source changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY . .
RUN touch src/main.rs \
    && cargo build --release

# Final stage
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /src/target/release/istio-proxy-terminator /istio-proxy-terminator

USER nonroot:nonroot

ENTRYPOINT ["/istio-proxy-terminator"]
