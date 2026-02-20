FROM rust:1.77-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build all service binaries
RUN cargo build --release \
    --bin service-gateway \
    --bin service-write \
    --bin service-retrieve \
    --bin service-graph

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/service-gateway /usr/local/bin/
COPY --from=builder /app/target/release/service-write /usr/local/bin/
COPY --from=builder /app/target/release/service-retrieve /usr/local/bin/
COPY --from=builder /app/target/release/service-graph /usr/local/bin/
COPY migrations/ /app/migrations/

ENV RUST_LOG=info
EXPOSE 8080 8081 8082 8083

CMD ["service-gateway"]
