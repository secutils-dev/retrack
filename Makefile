COMPOSE_DB   := dev/docker/docker-compose.yml
ENV_FILE     := .env
CHROME_PATH  ?= /Applications/Google Chrome.app/Contents/MacOS/Google Chrome

.PHONY: dev-up dev-down api scraper-setup scraper scraper-debug db-reset db-migrate test test-api test-scraper fmt clippy check docker-api docker-scraper docker-scraper-camoufox clean help

## ---------- Development ----------

dev-up: ## Start dev infrastructure (PostgreSQL). Use BUILD=1 to rebuild images.
	docker compose -f $(COMPOSE_DB) --env-file $(ENV_FILE) up $(if $(BUILD),--build --force-recreate) -d

dev-down: ## Stop dev infrastructure and remove volumes.
	docker compose -f $(COMPOSE_DB) --env-file $(ENV_FILE) down --volumes --remove-orphans

dev-logs: ## Tail logs from dev infrastructure.
	docker compose -f $(COMPOSE_DB) logs -f

api: ## Run the Retrack API on the host.
	cargo run

scraper-setup: ## Install web scraper npm dependencies (run once).
	npm install

scraper: ## Run web scraper on host (headed browser).
	RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_NO_HEADLESS=true \
	RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_EXECUTABLE_PATH="$(CHROME_PATH)" \
	npm run watch -w components/retrack-web-scraper

scraper-debug: ## Run web scraper with Playwright protocol debug output.
	DEBUG=pw:protocol \
	RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_NO_HEADLESS=true \
	RETRACK_WEB_SCRAPER_BROWSER_CHROMIUM_EXECUTABLE_PATH="$(CHROME_PATH)" \
	npm run watch -w components/retrack-web-scraper

## ---------- Testing ----------

test: test-api test-scraper ## Run all tests (API + Web Scraper).

test-api: ## Run Rust API tests.
	cargo test --all -- --nocapture

test-scraper: ## Run Web Scraper (Node.js) tests.
	npm test -w components/retrack-web-scraper

## ---------- Code Quality ----------

fmt: ## Check Rust formatting (requires nightly).
	cargo +nightly fmt --all -- --check

clippy: ## Run Clippy lints.
	cargo clippy --all --all-targets -- -D warnings

check: fmt clippy test ## Run format check, Clippy, and all tests.

## ---------- Database ----------

db-reset: ## Drop, create, and migrate the dev database.
	cargo sqlx database drop -y
	cargo sqlx database create
	cargo sqlx migrate run

db-migrate: ## Run pending database migrations.
	cargo sqlx migrate run

db-prepare: ## Regenerate the offline SQLx query cache (.sqlx/).
	cargo sqlx prepare

db-prepare-check: ## Verify the offline SQLx query cache is up to date.
	cargo sqlx prepare --check

## ---------- Docker Images ----------

docker-api: ## Build the Retrack API Docker image.
	docker build --tag retrack-api:latest .

docker-scraper: ## Build the Web Scraper (Chromium) Docker image.
	docker build --tag retrack-web-scraper:latest -f Dockerfile.web-scraper .

docker-scraper-camoufox: ## Build the Web Scraper (Camoufox/Firefox) Docker image.
	docker build --tag retrack-web-scraper-camoufox:latest -f Dockerfile.web-scraper-camoufox .

## ---------- Misc ----------

clean: ## Remove build artifacts.
	cargo clean
	rm -rf components/retrack-web-scraper/dist

help: ## Show this help message.
	@grep -E '^[a-zA-Z0-9_-]+:.*## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-24s\033[0m %s\n", $$1, $$2}'
