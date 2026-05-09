FROM rust:1.95-slim-trixie@sha256:a6ed0cbc27f063c367aee8373f35fdf4dcf8be7596c4d19d6643e3252e514c2e AS builder
WORKDIR /app

ARG TARGETARCH
ARG UPX_VERSION=5.1.1

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
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:8f960b7fc6a5d6e28bb07f982655925d6206678bd9a6cde2ad00ddb5e2077d78
EXPOSE 7676

WORKDIR /app
COPY --from=builder ["/app/retrack", "./"]

CMD [ "./retrack" ]
