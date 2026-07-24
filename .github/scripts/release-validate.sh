#!/usr/bin/env bash
#
# release-validate.sh — REL-001 (Issue #97) pre-flight validator.
#
# Enforces the "tag-only, clean, consistent, no-proxy-dep" release gate
# BEFORE any `cargo publish --dry-run` runs. The 0.1.2 accident was a
# dirty, tag-less, version-drifted publish; this script makes each of
# those three failure modes a hard, machine-checked refusal.
#
# Invoked by .github/workflows/release.yml on a `v*` tag push. It can also
# be run locally to rehearse a release:
#   GITHUB_REF=refs/tags/v0.1.1 ./.github/scripts/release-validate.sh
#
# It performs NO network calls and NO publish. Exit non-zero = refuse.

set -uo pipefail

ref="${GITHUB_REF:-refs/heads/main}"

# ---------------------------------------------------------------------------
# Gate 1 (DoD negative #1): must be triggered by a version tag, never a branch.
# ---------------------------------------------------------------------------
if [[ "$ref" != refs/tags/v* ]]; then
  echo "::error::Release pipeline must run from a tag push (refs/tags/vX.Y.Z), got: $ref"
  exit 1
fi
tag="${ref#refs/tags/}"
version="${tag#v}"   # strip the leading 'v'

# ---------------------------------------------------------------------------
# Gate 2 (DoD negative #2): four-way version consistency.
# tag == Cargo.toml version == capabilities.json version == CHANGELOG section.
# ---------------------------------------------------------------------------
# Strip a trailing CR so the script is robust to both CRLF (local Windows
# checkouts) and LF (CI) line endings — a bare \r would otherwise break the
# version-equality comparison.
cargo_version="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/' | tr -d '\r')"
caps_version="$(grep -m1 '"version"' llmrust.capabilities.json | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/' | tr -d '\r')"
changelog_ok=0
if grep -Eq "^## \\[${version}\\]" CHANGELOG.md; then
  changelog_ok=1
fi

fail=0
if [[ -z "$version" ]]; then
  echo "::error::Could not parse version from tag $tag"
  fail=1
fi
if [[ "$cargo_version" != "$version" ]]; then
  echo "::error::Cargo.toml version '$cargo_version' != tag version '$version'"
  fail=1
fi
if [[ "$caps_version" != "$version" ]]; then
  echo "::error::llmrust.capabilities.json version '$caps_version' != tag version '$version'"
  fail=1
fi
if [[ "$changelog_ok" -ne 1 ]]; then
  echo "::error::CHANGELOG.md has no section for [$version]"
  fail=1
fi
if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

# ---------------------------------------------------------------------------
# Gate 3 (DoD negative #3): clean working tree. No dirty release, ever.
# ---------------------------------------------------------------------------
if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  echo "::error::Working tree is dirty; release must run from a clean tag checkout."
  git status --porcelain --untracked-files=all
  exit 1
fi

# ---------------------------------------------------------------------------
# Gate 4 (DoD step 5): default feature must NOT pull in proxy-only deps.
# axum / tower-http / bytes are optional and only compiled under `proxy`.
# `cargo tree -i <dep>` exits non-zero when the dep is absent from the
# default graph -> that is the PASS condition here.
# ---------------------------------------------------------------------------
for dep in axum tower-http bytes; do
  if cargo tree -e no-dev -i "$dep" >/dev/null 2>&1; then
    echo "::error::default feature pulls in proxy-only dependency: $dep"
    exit 1
  fi
done

echo "release-validate: pre-flight checks passed for $tag (version $version)"
