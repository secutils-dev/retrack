FROM rust:1.99-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 AS builder
WORKDIR /app

ARG TARGETARCH
ARG UPX_VERSION=5.3.0

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
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e
EXPOSE 7676

WORKDIR /app
COPY --from=builder ["/app/retrack", "./"]

CMD [ "./retrack" ]
