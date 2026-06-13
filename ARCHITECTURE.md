# Retrack Architecture

This document provides an overview of the Retrack architecture.

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Repository Structure](#repository-structure)
- [Dependencies](#dependencies)
- [API Endpoints](#api-endpoints)
- [Further Reading](#further-reading)

## Overview

Retrack is an open-source service that tracks changes in web pages, APIs, and files. It provides:

- **Page Tracking**: Monitor web page content via headless browsers (Chromium or Camoufox/Firefox) with custom JavaScript extractor scripts
- **API Tracking**: Monitor HTTP/REST API responses with optional configurator and extractor scripts
- **File Tracking**: Parse and monitor CSV and XLS/XLSX files for changes
- **Change Detection**: Compare revisions using unified diffs and trigger actions (server log, email, webhook) on detected changes
- **Scheduled Checks**: Cron-based scheduling with configurable intervals and retry strategies
- **Notifications**: Email and webhook notifications when changes are detected or checks fail

The Retrack API is written in **Rust** using the **Actix-web** framework, with **PostgreSQL** for data persistence and an embedded **Deno/V8** JavaScript runtime for user-defined scripts. The Web Scraper component is a **Node.js** service using **Playwright** to drive headless browsers.

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                                         Clients                                         │
│                                                                                         │
│                           ┌──────────────┐    ┌──────────────┐                          │
│                           │   REST API   │    │   RapiDoc    │                          │
│                           │   Clients    │    │   (OpenAPI)  │                          │
│                           └──────┬───────┘    └──────┬───────┘                          │
│                                  │                   │                                  │
└──────────────────────────────────┼───────────────────┼──────────────────────────────────┘
                                   │                   │
                                   └───────────────────┘
                                             │
                                             ▼
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│                                   Retrack API Server                                    │
│                                     (Port: 7676)                                        │
│                                                                                         │
│   ┌─────────────────────────────────────────────────────────────────────────────────┐   │
│   │                              HTTP Server (Actix-web)                            │   │
│   │                                                                                 │   │
│   │   ┌──────────────┐  ┌──────────────────┐  ┌─────────────────────────────────┐   │   │
│   │   │ /api/status  │  │ /api/trackers    │  │ /api/trackers/{id}/revisions    │   │   │
│   │   └──────────────┘  └──────────────────┘  └─────────────────────────────────┘   │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                            │                                            │
│   ┌────────────────────────────────────────┴────────────────────────────────────────┐   │
│   │                                 API Layer (api.rs)                              │   │
│   │                                                                                 │   │
│   │   ┌──────────────┐  ┌──────────────┐  ┌───────────────┐  ┌──────────────────┐   │   │
│   │   │   Trackers   │  │    Tasks     │  │   Scheduler   │  │     Error        │   │   │
│   │   │   API Ext    │  │   API Ext    │  │    API Ext    │  │    Handling      │   │   │
│   │   └──────────────┘  └──────────────┘  └───────────────┘  └──────────────────┘   │   │
│   └─────────────────────────────────────────────────────────────────────────────────┘   │
│                                            │                                            │
│   ┌────────────────────────────────────────┴─────────────────────────────────────────┐  │
│   │                              Core Services                                       │  │
│   │                                                                                  │  │
│   │          ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐          │  │
│   │          │ Scheduler  │  │ JS Runtime │  │  Network   │  │ Templates  │          │  │
│   │          │  (Cron)    │  │ (Deno/V8)  │  │ (HTTP/DNS) │  │(Handlebars)│          │  │
│   │          └────────────┘  └────────────┘  └────────────┘  └────────────┘          │  │
│   └──────────────────────────────────────────────────────────────────────────────────┘  │
│                                            │                                            │
└────────────────────────────────────────────┼────────────────────────────────────────────┘
                                             │
                           ┌─────────────────┼─────────────────┐
                           │                 │                 │
                           ▼                 ▼                 ▼
                  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
                  │  PostgreSQL  │  │  Web Scraper │  │  SMTP Server │
                  │   Database   │  │  (Chromium)  │  │   (Email)    │
                  │   Port: 5432 │  │  Port: 7272  │  │              │
                  └──────────────┘  └──────────────┘  └──────────────┘
```

## Repository Structure

```
retrack/
├── assets/                           # Project logo and Handlebars email templates (.hbs)
├── components/
│   ├── retrack-types/                # Shared Rust types (Cargo workspace member)
│   └── retrack-web-scraper/          # Web Scraper service (Node.js + Playwright)
├── dev/
│   ├── api/                          # HTTP client files for manual API testing
│   ├── docker/                       # Docker Compose (postgres.yml) and configs
├── migrations/                       # SQLx database migrations
├── src/                              # Retrack API server (Rust)
├── Cargo.toml                        # Rust workspace manifest
├── Dockerfile                        # Retrack API image (distroless)
├── Dockerfile.web-scraper            # Web Scraper image (Chromium + Xvfb)
├── Dockerfile.web-scraper-camoufox   # Web Scraper image (Camoufox/Firefox)
└── Makefile                          # Common development commands
```

## Dependencies

| Component           | Purpose                              | Technology              |
|---------------------|--------------------------------------|-------------------------|
| **PostgreSQL**      | Primary data store                   | SQL database (v16)      |
| **Web Scraper**     | Headless browser page rendering      | Node.js + Playwright    |
| **Deno/V8**         | User-defined script execution        | Embedded JS runtime     |
| **SMTP** (optional) | Email notifications and error alerts | Lettre (Rust)           |

## API Endpoints

| Method   | Path                                    | Description                      |
|----------|-----------------------------------------|----------------------------------|
| `GET`    | `/api/status`                           | Server status and version        |
| `GET`    | `/api/trackers`                         | List trackers with pagination    |
| `POST`   | `/api/trackers`                         | Create a new tracker             |
| `GET`    | `/api/trackers/{id}`                    | Get a tracker by ID              |
| `PUT`    | `/api/trackers/{id}`                    | Update a tracker                 |
| `DELETE` | `/api/trackers/{id}`                    | Remove a tracker                 |
| `POST`   | `/api/trackers/_bulk_get`               | Bulk get trackers by ID          |
| `DELETE` | `/api/trackers`                         | Bulk remove trackers by tags     |
| `POST`   | `/api/trackers/_debug`                  | Debug a new tracker definition   |
| `POST`   | `/api/trackers/{id}/_debug`             | Debug an existing tracker        |
| `GET`    | `/api/trackers/{id}/revisions`          | List revisions for a tracker     |
| `POST`   | `/api/trackers/{id}/revisions`          | Trigger an ad-hoc revision       |
| `GET`    | `/api/trackers/{id}/revisions/{rev_id}` | Get a specific revision          |
| `DELETE` | `/api/trackers/{id}/revisions/{rev_id}` | Remove a specific revision       |
| `DELETE` | `/api/trackers/{id}/revisions`          | Clear all revisions              |
| `GET`    | `/api/trackers/{id}/execution-logs`     | List execution logs for tracker  |
| `DELETE` | `/api/trackers/{id}/execution-logs`     | Clear execution logs for tracker |
| `DELETE` | `/api/trackers/execution-logs`          | Clear all execution logs         |

OpenAPI documentation is served at `/api-docs` via RapiDoc.

## Further Reading

- [Playwright Documentation](https://playwright.dev/)
- [Deno Core](https://github.com/denoland/deno/tree/main/libs/core)
