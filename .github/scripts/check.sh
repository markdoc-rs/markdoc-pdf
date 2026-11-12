#!/usr/bin/env bash
#
# Everything that has to be true before markdoc-pdf is published.
#
# ONE COPY, as in markdoc-lint, markdoc-ls, markdoc-prose and tw: ci.yml
# runs it on every push and pull request, on Linux, macOS and Windows, and
# release.yml runs it before anything is published. A second list of steps
# drifts, and the half that drifts is always the one nobody watches.
#
# It expects what ci.yml installs first: on Linux, fontconfig's headers, and
# two font families the per-element font tests can tell apart.
#
# Inputs:
#   $1          the markdoc-pdf checkout               default: $PWD
#   STRICT      1 to fail on clippy warnings           default: 1
set -euo pipefail

ROOT="${1:-$PWD}"
cd "$ROOT"
ROOT="$PWD"
STRICT="${STRICT:-1}"

group() { echo "::group::$*"; }
endgroup() { echo "::endgroup::"; }
die() { echo "::error::$*" >&2; exit 1; }

[ -f Cargo.toml ] || die "no Cargo.toml in $ROOT — point this at the markdoc-pdf checkout"

failed=""
step() {
  local name="$1"; shift
  group "$name"
  if "$@"; then
    endgroup
  else
    endgroup
    echo "::error::$name failed"
    failed="$failed $name"
  fi
}

# Formatting first: it is the cheapest, and a reformat in the middle of a
# clippy fix is a confusing diff to review.
step "cargo fmt" cargo fmt --all --check

# Cargo.lock is committed, and has to agree with Cargo.toml: the release
# publishes with `cargo publish --locked`, which refuses one that does not,
# and finding that out here costs a push rather than a release.
lock_is_current() { cargo metadata --locked --format-version 1 >/dev/null; }
step "Cargo.lock is current" lock_is_current

# Every other step builds with `mermaid`, a default feature. The second
# clippy keeps the build without it compiling and warning-free too, since
# the `cfg` gates differ there.
if [ "$STRICT" = "1" ]; then
  step "cargo clippy" cargo clippy --all-targets -- -D warnings
  step "cargo clippy --no-default-features" cargo clippy --all-targets --no-default-features -- -D warnings
else
  step "cargo clippy" cargo clippy --all-targets
  step "cargo clippy --no-default-features" cargo clippy --all-targets --no-default-features
fi

step "cargo test" cargo test --all-targets

step "cargo build --release" cargo build --release --bin markdoc-pdf

# A binary that starts and writes nothing is the failure that matters to a
# reader, and examples/rich.mdoc reaches most of what the renderer lays
# out. The PDF goes to a temporary directory, not into the checkout.
out="$(mktemp -d)"
step "it renders" sh -c "./target/release/markdoc-pdf --input examples/rich.mdoc \
  --output '$out/rich.pdf' --style examples/themes/letter.style.toml \
  --assets-root examples && test -s '$out/rich.pdf'"
rm -rf "$out"

for s in .github/scripts/*.sh; do
  step "bash -n $(basename "$s")" bash -n "$s"
done

if [ -n "$failed" ]; then
  die "failed:$failed"
fi
echo "==> all checks passed"
