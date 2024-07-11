# syntax=docker/dockerfile:1

FROM --platform=$BUILDPLATFORM node:20-alpine3.20 AS builder
WORKDIR /app
COPY ["./*.json", "./"]
RUN set -x && npm ci
COPY ["./src", "./src"]
RUN set -x && npm test
RUN set -x && npm run build

FROM node:20-alpine3.20
ENV NODE_ENV=production \
    RETRACK_WEB_SCRAPER_BROWSER_EXECUTABLE_PATH="/usr/bin/chromium-browser"
WORKDIR /app
RUN set -x && apk update --no-cache && \
    apk upgrade --no-cache && \
    apk add --no-cache dumb-init nss freetype harfbuzz ca-certificates ttf-freefont chromium
COPY --from=builder ["/app/dist", "/app/package.json", "/app/package-lock.json", "./"]
RUN set -x && npm ci && npm cache clean --force
USER node
CMD [ "node", "src/index.js" ]
