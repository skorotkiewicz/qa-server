# Build stage
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations

# Build release binaries
RUN cargo build --release --locked

# Runtime stage
FROM alpine:latest

RUN apk add --no-cache ca-certificates libgcc

WORKDIR /app

# Copy binaries
COPY --from=builder /app/target/release/qa-server /usr/local/bin/qa-server
COPY --from=builder /app/target/release/qa /usr/local/bin/qa
COPY --from=builder /app/migrations /app/migrations

# Create config and data directories
RUN mkdir -p /data /etc/qa-server

# Copy default config (can be overridden by volume)
COPY server.yml.example /etc/qa-server/server.yml

# Environment variables with defaults
ENV DATABASE_URL=sqlite:///data/qa.db
ENV RUST_LOG=info
ENV QA_SERVER_IP=0.0.0.0
ENV QA_SERVER_PORT=7879

EXPOSE 7879

VOLUME ["/data"]

# Use config from /etc/qa-server by default
CMD ["qa-server", "/etc/qa-server/server.yml"]
