# Builder stage
FROM rust:bookworm AS builder

WORKDIR /var/dashboard

COPY . .

RUN cargo install --path .

# Path: DockerFile
FROM ubuntu:22.04

RUN apt-get update && apt-get install -y libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/backend /usr/local/bin/dashboard

CMD ["dashboard"]
