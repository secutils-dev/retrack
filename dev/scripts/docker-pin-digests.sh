#!/usr/bin/env bash
#
# Re-pin base images in Retrack Dockerfiles to their current SHA256 manifest-list digests.
# Usage: ./dev/scripts/docker-pin-digests.sh
#
set -euo pipefail

RETRACK_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

DOCKERFILES=(
  "$RETRACK_ROOT/Dockerfile"
  "$RETRACK_ROOT/Dockerfile.web-scraper"
  "$RETRACK_ROOT/Dockerfile.web-scraper-camoufox"
)

# File-based cache so subshells can share resolved digests.
CACHE_FILE="$(mktemp)"
trap 'rm -f "$CACHE_FILE"' EXIT

get_digest() {
  local image="$1"

  # Check cache.
  local cached
  cached="$(grep -F "$image " "$CACHE_FILE" 2>/dev/null | head -1 | awk '{print $2}')" || true
  if [[ -n "$cached" ]]; then
    echo "$cached"
    return
  fi

  echo "  Fetching digest for $image ..." >&2
  local digest
  digest="$(docker buildx imagetools inspect "$image" 2>/dev/null \
    | grep -m1 '^Digest:' | awk '{print $2}')"

  if [[ -z "$digest" || ! "$digest" =~ ^sha256: ]]; then
    echo "ERROR: failed to fetch digest for $image" >&2
    return 1
  fi

  # Strip the sha256: prefix  we add it back when rewriting.
  digest="${digest#sha256:}"
  echo "$image $digest" >> "$CACHE_FILE"
  echo "$digest"
}

for dockerfile in "${DOCKERFILES[@]}"; do
  [[ -f "$dockerfile" ]] || { echo "SKIP: $dockerfile not found"; continue; }

  tmp="$(mktemp)"
  changed=false

  while IFS= read -r line; do
    if [[ "$line" =~ ^FROM[[:space:]] ]]; then
      # Strip any existing @sha256:... from the image reference.
      stripped="$(echo "$line" | sed -E 's/@sha256:[0-9a-f]+//')"

      # Extract the image:tag - it's the token after FROM (and optional --platform=...).
      image_tag="$(echo "$stripped" | sed -E 's/^FROM[[:space:]]+(--platform=[^ ]+[[:space:]]+)?([^ ]+).*/\2/')"

      digest="$(get_digest "$image_tag")"
      # Insert @sha256:digest right after the image:tag in the stripped line.
      line="$(echo "$stripped" | sed -E "s|${image_tag}|${image_tag}@sha256:${digest}|")"
      changed=true
    fi
    echo "$line"
  done < "$dockerfile" > "$tmp"

  if $changed; then
    mv "$tmp" "$dockerfile"
    echo "Pinned: $dockerfile"
  else
    rm "$tmp"
    echo "SKIP: no FROM lines in $dockerfile"
  fi
done
