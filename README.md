# <img src="https://raw.githubusercontent.com/secutils-dev/retrack/main/assets/logo/retrack-logo-initials.png" alt="Retrack" width="22"> [Retrack](https://retrack.dev) &middot; [![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://github.com/secutils-dev/retrack/blob/main/LICENSE) [![Build Status](https://github.com/secutils-dev/retrack/actions/workflows/ci.yml/badge.svg)](https://github.com/secutils-dev/retrack/actions)

Retrack - track changes in a web page, API, or file.

## Getting started

Before running the Retrack server, you need to configure the database connection. If you don't have a PostgreSQL server
running,
you [can run it locally with the following Docker Compose file:](https://docs.docker.com/language/rust/develop/)

```shell
docker-compose -f ./dev/docker/postgres.yml up --build --force-recreate
```

To remove everything and start from scratch, run:

```shell
docker-compose -f ./dev/docker/postgres.yml down --volumes --remove-orphans
```

Make sure to replace `POSTGRES_HOST_AUTH_METHOD=trust` in Docker Compose file with a more secure authentication method
if you're
planning to use a local database for an extended period. For the existing database, you'll need to provide connection
details in the
TOML configuration file as explained below.

Once all services are configured, you can start the Retrack server with `cargo run`. By default, the
server will be accessible via http://localhost:7676. Use `curl` to verify that the server is up and running:

```shell
curl -XGET http://localhost:7676/api/status
---
{"version":"0.0.1"}
```

The server can be configured with a TOML configuration file. See the example below for a basic configuration:

```toml
port = 7676

[db]
name = 'retrack'
host = 'localhost'
port = 5432
username = 'postgres'
password = 'password'

# Connection details for Web Scraper service.
[components]
web_scraper_url = 'http://localhost:7272/'

# SMTP server configuration used to send emails (signup emails, notifications etc.).
[smtp]
address = "xxx"
username = "xxx"
password = "xxx"

# Trackers specific configuration.
[trackers]
max_revisions = 10
min_schedule_interval = 600_000
schedules = ["@", "@hourly", "@daily", "@weekly", "@monthly", "@@"]
```

If you saved your configuration to a file named `retrack.toml`, you can start the server with the following command:

```shell
cargo run -- -c retrack.toml
```

You can also use `.env` file to specify the location of the configuration file and database connection details required
for development and testing:

```dotenv
# Refer to https://github.com/launchbadge/sqlx for more details.
DATABASE_URL=postgres://postgres@localhost/retrack

# Path to the configuration file.
RETRACK_CONFIG=${PWD}/retrack.toml
```
