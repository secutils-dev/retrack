FROM rust:1.99-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581 AS builder
WORKDIR /app

ARG TARGETARCH
ARG UPX_VERSION=5.2.1

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
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:e792ab3d241a468a4fd7519ddbbebe66b49b5f365771716ea688ad40b6c6f1c2
EXPOSE 7676

WORKDIR /app
COPY --from=builder ["/app/retrack", "./"]

CMD [ "./retrack" ]
