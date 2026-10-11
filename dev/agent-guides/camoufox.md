# Camoufox image and runtime

Run build commands from the Retrack root.
Read `Dockerfile.web-scraper-camoufox` and `dev/docker/camoufox_launcher.py` before changes.
Read actual package and browser pins from the Dockerfile.

## Coupled versions

Treat the official PyPI `camoufox` wrapper, PyPI `playwright`, and Firefox `CAMOUFOX_VERSION` as a compatibility set.
Inspect the wrapper's Playwright requirement before the upgrade.
Install both Python packages together and run `pip check` inside the image.
For Secutils integration, match the Node-side Playwright minor in scraper, Web UI, and E2E.

Use the official `camoufox` package and Playwright's public driver entrypoint.
The `cloverlabs-camoufox` fork requires the removed `lib/browserServerImpl.js` internal and can crash with `MODULE_NOT_FOUND`.
Install `playwright` from PyPI, rather than a Git ref.
The official wrapper supports `fontconfig/` and `fontconfigs/` without the former local patch.

For Python versions without an `indexed-zstd` wheel, install `build-essential` and `libzstd-dev` before pip.
This is required by the wrapper's `fpgen` dependency on Python 3.14 arm64.
Purge build dependencies in the same layer and retain runtime `libzstd1`.

## Browser resolution and cache

Pin the Firefox version in `CAMOUFOX_VERSION`, rather than a full release/build label.
For a requested release whose asset labels match across architectures, also pin `CAMOUFOX_BUILD_LABEL`.
Resolve its build from the platform-filtered `repo_cache.json` after `camoufox sync`.
Release tags and asset labels can differ. Asset labels can also differ between arm64 and amd64.
A full label that works locally can therefore fail CI's other architecture.

Resolve the pinned version explicitly rather than using `official/stable`.
After fetching, select it with `camoufox set official/prerelease/<version>-<build>` for prereleases.
Verify `camoufox active` inside the runtime image matches the requested build.
The sync cache deduplicates build labels, so an older version can disappear when a newer release reuses its label.
Label sorting can also rank an older beta above a newer alpha.

Keep the browser BuildKit cache mount across rebuilds.
Allow disk space for the large archive, extracted browser, and image export.
To rerun a layer, change its relevant input and preserve cache mounts.
**Avoid `--no-cache` for this image** so expensive browser downloads remain reusable.
Copy the installed cache into the runtime image because a BuildKit cache mount is not image content.

### Silent upstream failures

- `camoufox sync` can report GitHub HTTP 403, write an empty cache, and exit successfully.
  The wrapper supports `GITHUB_TOKEN` for authenticated requests.
  A browser version change does not resolve the API rate limit.
- `camoufox fetch official/<id>` can exit successfully without an available build.
  Preserve the Dockerfile's non-empty resolved-build assertion.
- Installed browser directories include a SHA suffix after the version/build ID.
  Locate the directory by the resolved ID prefix and require its `version.json` marker.
  A directory existence check alone does not establish a usable installation.

If runtime reports `CamoufoxNotInstalled`, inspect the resolved build and installed marker before a re-pin/rebuild.

## Completion gate

1. Build with `make docker-scraper-camoufox`.
2. Inside the image, require passing `pip check`, the requested Python version, launcher, and installed browser marker.
3. For integration in Secutils, run these commands from the parent Secutils root:

   ```bash
   make e2e-up BUILD=1 ONLY=retrack_web_scraper_camoufox
   make e2e-test ARGS="tests/page_tracker_camoufox.spec.ts"
   ```

4. Require every test in that spec to pass, including the Chromium control for the Gecko-only feature.

Retrack's local Compose file starts only PostgreSQL.
If the parent stack is unavailable, report the integration gate as pending.
