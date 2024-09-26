FROM rust:1.81-slim-bookworm AS builder
WORKDIR /app

# Install dependencies.
RUN set -x && apt-get update && apt-get install -y protobuf-compiler

# Copy manifest and fetch dependencies.
COPY ["./Cargo.lock", "./Cargo.toml", "./"]
RUN set -x && cargo fetch

# Copy source code and build.
COPY [".", "./"]
RUN --mount=type=cache,target=/app/target cargo build --release && cp ./target/release/retrack ./

FROM gcr.io/distroless/cc-debian12:nonroot
EXPOSE 7676

WORKDIR /app
COPY --from=builder ["/app/retrack", "./"]

CMD [ "./retrack" ]
