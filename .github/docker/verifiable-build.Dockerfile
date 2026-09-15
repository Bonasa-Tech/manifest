# Upstream's verifiable-build image with one thing added: the platform-tools
# release this repository pins.
#
# The image installs agave 2.2.20 (built-in platform-tools v1.48) and warms its
# toolchain cache by running `cargo build-sbf` with no --tools-version, so v1.48
# is the only version present. Asking that container for v1.57 therefore misses
# both short-circuits in cargo-build-sbf's resolution - requested != built-in,
# and no cached version is >= requested - and falls through to
# platform-tools/releases/latest. GitHub reports "latest" as the most recently
# published release rather than the highest version, so since upstream published
# v1.51.1 that pointer goes backwards, v1.57 is judged invalid, and the build
# silently falls back to v1.48 (rustc 1.84.1 instead of 1.95).
#
# Seeding the cache here makes the second short-circuit hit, so the requested
# version is the version used and the result does not depend on what upstream
# publishes next. Nothing else about the base image changes: same digest, same
# compiler once resolution lands where it should.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}

ARG PLATFORM_TOOLS
ARG PLATFORM_TOOLS_SHA256
ARG PLATFORM_TOOLS_TARBALL=platform-tools-linux-x86_64.tar.bz2

RUN set -eux; \
    dest="/root/.cache/solana/${PLATFORM_TOOLS}/platform-tools"; \
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
      --output "/tmp/${PLATFORM_TOOLS_TARBALL}" \
      "https://github.com/anza-xyz/platform-tools/releases/download/${PLATFORM_TOOLS}/${PLATFORM_TOOLS_TARBALL}"; \
    echo "${PLATFORM_TOOLS_SHA256}  /tmp/${PLATFORM_TOOLS_TARBALL}" | sha256sum --check --strict; \
    mkdir -p "${dest}"; \
    tar -xjf "/tmp/${PLATFORM_TOOLS_TARBALL}" -C "${dest}"; \
    rm -f "/tmp/${PLATFORM_TOOLS_TARBALL}"; \
    "${dest}/rust/bin/rustc" --version
