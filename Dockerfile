FROM rust:1.93-slim-trixie@sha256:c0a38f5662afdb298898da1d70b909af4bda4e0acff2dc52aea6360a9b9c6956 AS builder
WORKDIR /app

ARG TARGETARCH
ARG UPX_VERSION=5.1.0

# Install dependencies.
RUN set -x && apt-get update && apt-get install -y protobuf-compiler curl xz-utils

# Download and install UPX.
RUN curl -LO https://github.com/upx/upx/releases/download/v${UPX_VERSION}/upx-${UPX_VERSION}-${TARGETARCH}_linux.tar.xz && \
    tar -xf upx-${UPX_VERSION}-${TARGETARCH}_linux.tar.xz && \
    mv upx-${UPX_VERSION}-${TARGETARCH}_linux/upx /usr/local/bin/ && \
    rm -rf upx-${UPX_VERSION}-${TARGETARCH}_linux.tar.xz upx-${UPX_VERSION}-${TARGETARCH}_linux

# Copy source code and build.
COPY [".", "./"]
RUN --mount=type=cache,target=/app/target cargo build --release && \
    cp ./target/release/retrack ./ && \
    upx --best --lzma ./retrack

# Check out https://gcr.io/distroless/cc-debian13:nonroot
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:9c4fe2381c2e6d53c4cfdefeff6edbd2a67ec7713e2c3ca6653806cbdbf27a1e
EXPOSE 7676

WORKDIR /app
COPY --from=builder ["/app/retrack", "./"]

CMD [ "./retrack" ]
