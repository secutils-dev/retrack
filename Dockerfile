FROM rust:1.97-slim-trixie@sha256:34fb2f168c432d421a09883c663b275b33cbb30f6b18642fbd09a684c6546d0e AS builder
WORKDIR /app

ARG TARGETARCH
ARG UPX_VERSION=5.2.0

# Install dependencies.
RUN set -x && apt-get update && apt-get install -y make protobuf-compiler curl xz-utils

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
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:d97bc0a941b8d4be647dc0ee75b264ddbb772f1ac5ba690a4309c00723b23775
EXPOSE 7676

WORKDIR /app
COPY --from=builder ["/app/retrack", "./"]

CMD [ "./retrack" ]
