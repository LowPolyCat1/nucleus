#!/usr/bin/env bash
# One entry point for every check. Usage:
#   scripts/test.sh            # rust + frontend + e2e (no container engine needed)
#   scripts/test.sh rust       # fmt, clippy, cargo tests on the fake sandbox backend
#   scripts/test.sh frontend   # typecheck + vitest (jsdom, mock backend)
#   scripts/test.sh e2e        # playwright in headless chromium against the mock backend
#   scripts/test.sh engine     # tests against a real Podman/Docker engine
#   scripts/test.sh all        # everything, including engine
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }

rust() {
  step "cargo fmt --check"
  cargo fmt --all --check
  step "cargo clippy"
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  step "cargo test (fake sandbox backend)"
  cargo test --workspace --all-features
}

frontend_deps() {
  [ -d app/node_modules ] || (cd app && npm ci --no-audit --no-fund)
}

frontend() {
  frontend_deps
  step "typecheck"
  (cd app && npx tsc --noEmit)
  step "vitest"
  (cd app && npx vitest run)
}

e2e() {
  frontend_deps
  step "playwright (mock backend)"
  # Prefer a preinstalled chromium when Playwright's own browser is not downloaded.
  if [ -z "${PLAYWRIGHT_CHROMIUM_PATH:-}" ] && [ -x /opt/pw-browsers/chromium ] && [ ! -d "${HOME}/.cache/ms-playwright" ] && [ -z "${PLAYWRIGHT_BROWSERS_PATH:-}" ]; then
    export PLAYWRIGHT_CHROMIUM_PATH=/opt/pw-browsers/chromium
  fi
  (cd app && CI=1 npx playwright test)
}

engine() {
  step "engine tests (real Podman/Docker)"
  NUCLEUS_ENGINE_TESTS=1 cargo test --workspace --all-features -- --test-threads=4
}

case "${1:-default}" in
  rust) rust ;;
  frontend) frontend ;;
  e2e) e2e ;;
  engine) engine ;;
  all) rust; frontend; e2e; engine ;;
  default) rust; frontend; e2e ;;
  *) echo "unknown stage: $1" >&2; exit 2 ;;
esac
step "ok"
