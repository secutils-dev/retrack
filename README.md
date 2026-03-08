# <img src="https://raw.githubusercontent.com/secutils-dev/retrack/main/assets/logo/retrack-logo-initials.png" alt="Retrack" width="22"> [Retrack](https://retrack.dev) &middot; [![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://github.com/secutils-dev/retrack/blob/main/LICENSE) [![Build Status](https://github.com/secutils-dev/retrack/actions/workflows/ci.yml/badge.svg)](https://github.com/secutils-dev/retrack/actions)

Retrack tracks changes in web pages, APIs, and files. It runs scheduled checks, detects differences between revisions,
and triggers actions (server log, email, webhook) when content changes.

## Features

* **Page tracking** - render pages in a headless browser (Chromium or Camoufox/Firefox) and extract content with custom
  JavaScript scripts
* **API tracking** - monitor HTTP/REST API responses with optional configurator and extractor scripts
* **File tracking** - parse and monitor CSV and XLS/XLSX files for changes
* **Change detection** - unified diffs between revisions with configurable revision limits
* **Scheduled checks** - cron-based scheduling with retry strategies
* **Execution logs** - per-run execution history with structured timing phases, accessible via API
* **Notifications** - email and webhook notifications on detected changes or check failures
* **OpenAPI docs** - auto-generated specification served via RapiDoc at `/api-docs`

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)
- [Node.js](https://nodejs.org/) 22+ (see `.nvmrc`)
- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/)
- [sqlx-cli](https://github.com/launchbadge/sqlx/tree/main/sqlx-cli) (for database management)

## Getting Started

### 1. Clone the repository

```shell
git clone https://github.com/secutils-dev/retrack.git
cd retrack
```

### 2. Set up the environment

Copy the example environment file and customize it if needed:

```shell
cp .env.example .env
```

The default `.env` points to a local PostgreSQL instance:

```dotenv
DATABASE_URL=postgres://postgres@localhost/retrack
SQLX_OFFLINE=false
```

### 3. Start the infrastructure

Start PostgreSQL with Docker Compose:

```shell
make dev-up
```

To tear everything down and start fresh:

```shell
make dev-down
```

### 4. Start the Retrack API

```shell
make api
```

The API will be available at http://localhost:7676. Verify it is running:

```shell
curl -s http://localhost:7676/api/status
# {"version":"0.0.1"}
```

### 5. Start the Web Scraper (optional, for page trackers)

If you plan to use page trackers, install dependencies and start the Web Scraper:

```shell
make scraper-setup   # once: install npm dependencies
make scraper         # run with visible Chrome browser
```

The Web Scraper will be available at http://localhost:7272.

To run with Playwright protocol debug output:

```shell
make scraper-debug
```

## Configuration

The server is configured with a TOML file (`retrack.toml`). See the example below:

```toml
port = 7676

[db]
name = 'retrack'
host = 'localhost'
port = 5432
username = 'postgres'
password = 'password'

[components]
web_scraper_url = 'http://localhost:7272/'

[smtp]
address = "xxx"
username = "xxx"
password = "xxx"

[trackers]
max_revisions = 10
min_schedule_interval = 600_000
schedules = ["@", "@hourly", "@daily", "@weekly", "@monthly", "@@"]
# Retention period for execution logs in milliseconds (default: 90 days).
execution_log_retention = 7_776_000_000
```

Start the server with a custom config:

```shell
cargo run -- -c retrack.toml
```

You can also override configuration values via environment variables with the `RETRACK_` prefix
(nested keys use `__`, e.g. `RETRACK_DB__HOST=localhost`).

The Web Scraper is configured via environment variables:

```dotenv
RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_EXECUTABLE_PATH="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_NO_HEADLESS=true
RETRACK_WEB_SCRAPER_CACHE_TTL_SEC=5
```

## Re-initialize a local database

To manage the **development** database, install
[sqlx-cli](https://github.com/launchbadge/sqlx/tree/main/sqlx-cli):

```shell
cargo install --force sqlx-cli

# Drops, creates, and migrates the database
make db-reset
```

Or run the individual commands:

```shell
cargo sqlx database drop -y
cargo sqlx database create
cargo sqlx migrate run
```

## Docker

Build images with the following commands:

```shell
# Retrack API (distroless, port 7676)
make docker-api

# Web Scraper - Chromium backend (Alpine + Xvfb, port 7272)
make docker-scraper

# Web Scraper - Camoufox/Firefox backend (port 7777)
make docker-scraper-camoufox
```

Or directly:

```shell
docker build --tag retrack-api:latest .
docker build --tag retrack-web-scraper:latest -f Dockerfile.web-scraper .
docker build --tag retrack-web-scraper-camoufox:latest -f Dockerfile.web-scraper-camoufox .
```

## Available Make targets

| Command                        | Description                                            |
|--------------------------------|--------------------------------------------------------|
| `make dev-up`                  | Start dev infrastructure (`BUILD=1` to rebuild images) |
| `make dev-down`                | Stop dev infrastructure and remove volumes             |
| `make dev-logs`                | Tail logs from dev infrastructure                      |
| `make api`                     | Run the Retrack API (`cargo run`)                      |
| `make scraper-setup`           | Install web scraper npm dependencies (run once)        |
| `make scraper`                 | Run web scraper on host with visible browser           |
| `make scraper-debug`           | Run web scraper with Playwright protocol debug output  |
| `make test`                    | Run all tests (API + Web Scraper)                      |
| `make test-api`                | Run Rust API tests                                     |
| `make test-scraper`            | Run Web Scraper (Node.js) tests                        |
| `make fmt`                     | Check Rust formatting (requires nightly)               |
| `make clippy`                  | Run Clippy lints                                       |
| `make check`                   | Run format check, Clippy, and all tests                |
| `make db-reset`                | Drop, create, and migrate the dev database             |
| `make db-migrate`              | Run pending database migrations                        |
| `make db-prepare`              | Regenerate the offline SQLx query cache (`.sqlx/`)     |
| `make db-prepare-check`        | Verify the offline SQLx query cache is up to date      |
| `make docker-api`              | Build the Retrack API Docker image                     |
| `make docker-scraper`          | Build the Web Scraper (Chromium) Docker image          |
| `make docker-scraper-camoufox` | Build the Web Scraper (Camoufox/Firefox) Docker image  |
| `make clean`                   | Remove build artifacts                                 |
| `make help`                    | Show all available targets                             |

## Community

- ❓ Ask questions on [GitHub Discussions](https://github.com/secutils-dev/retrack/discussions)
- 🐛 Report bugs on [GitHub Issues](https://github.com/secutils-dev/retrack/issues)
