# syntax=docker/dockerfile:1.7

# The Teeny bot: the `teeny` binary on the OpenHuman embedded harness, the
# TinyComputer browser module beside it, and Chromium to drive.
#
#   docker build -t teeny .            # from the repo root, submodules in place
#
# vendor/openhuman has to be checked out recursively first
# (`git submodule update --init --recursive vendor/openhuman`): the cargo
# [patch] tables and the TinyComputer module both build from its nested
# vendors. The TypeScript service it replaces builds from legacy/Dockerfile
# with this same context.
#
# Layout of the result:
#   /usr/local/bin/teeny              the service and its subcommands
#   /usr/local/bin/teeny-entrypoint   docker/entrypoint.sh
#   /opt/tinycomputer/                libtinycomputer.so + modules.toml
#   /app/                             persona and content, read-only
#   /data/                            the bot-data volume (see the entrypoint)

# Keep in step with rust-toolchain.toml; the builder checks.
ARG RUST_VERSION=1.96.1

# ==========================================================================
# Build base: the toolchain and the system headers OpenHuman links against.
# ==========================================================================
FROM rust:${RUST_VERSION}-bookworm AS build-base
ARG RUST_VERSION
ENV DEBIAN_FRONTEND=noninteractive \
    # The image's own toolchain, not a rustup download of the one
    # rust-toolchain.toml names (with its rustfmt and clippy) on every build.
    RUSTUP_TOOLCHAIN=${RUST_VERSION}
# cpal, enigo, arboard and rdev are unconditional in the OpenHuman graph and
# link ALSA, X11/xdo/Xtst/Xi, evdev and udev even though a headless server
# never calls them. cmake and clang cover the -sys crates that build C.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake clang pkg-config ca-certificates \
        libssl-dev libasound2-dev libxdo-dev libxtst-dev libx11-dev \
        libxi-dev libevdev-dev libudev-dev \
    && rm -rf /var/lib/apt/lists/*

# ==========================================================================
# teeny
# ==========================================================================
FROM build-base AS teeny
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN channel="$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)" \
 && [ "$channel" = "$RUST_VERSION" ] \
 || { echo "rust-toolchain.toml pins $channel, the image is $RUST_VERSION: bump RUST_VERSION" >&2; exit 1; }
COPY vendor/openhuman ./vendor/openhuman
COPY crates ./crates
# The target dir is a cache mount, so the binary is copied out of it in the
# same step. Registry and git caches survive between builds too.
RUN --mount=type=cache,id=teeny-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=teeny-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=teeny-target,target=/src/target \
    cargo build --release --locked -p teeny \
 && install -Dm755 target/release/teeny /out/teeny

# ==========================================================================
# The TinyComputer module, from the same vendored SHA as the core: the host
# refuses a module whose contract version does not match its own.
# ==========================================================================
FROM build-base AS tinycomputer
WORKDIR /src/tinycomputer
COPY vendor/openhuman/vendor/tinycomputer/ ./
# build-module installs libtinycomputer.so (644) and the modules.toml that
# attests its SHA-256 into a 755 directory, and checks that every directory
# above it is root-owned and not group- or world-writable, which is what the
# TinyBus loader demands. /opt/tinycomputer passes.
ENV TINYCOMPUTER_MODULE_DIR=/opt/tinycomputer
RUN --mount=type=cache,id=teeny-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=teeny-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=tinycomputer-target,target=/src/tinycomputer/target \
    scripts/build-module

# ==========================================================================
# Runtime
# ==========================================================================
FROM debian:bookworm-slim AS runtime
ENV DEBIAN_FRONTEND=noninteractive
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
# chromium for the browser module, and the fonts it and the chart renderer
# draw text with (without them every label silently disappears). tini reaps
# Chromium's children; gosu drops root after the entrypoint's heal. The rest
# are the runtime halves of the build-base headers.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates tini gosu \
        chromium fonts-noto-core fonts-noto-color-emoji \
        libssl3 libasound2 libxdo3 libxtst6 libx11-6 libxi6 libevdev2 libudev1 \
    && rm -rf /var/lib/apt/lists/*

# A fixed uid, so the volume's ownership survives rebuilds. HOME is on the
# volume because OpenHuman keeps its keyring and skill registry under it.
RUN groupadd --gid 10001 teeny \
 && useradd --uid 10001 --gid 10001 --no-create-home \
        --home-dir /data/openhuman/home --shell /usr/sbin/nologin teeny

COPY --from=teeny /out/teeny /usr/local/bin/teeny
COPY --from=tinycomputer /opt/tinycomputer/ /opt/tinycomputer/
# Root-owned, 755 down to the library, 644 on the library and its
# attestation: the loader's rules. Kept together because a library without
# the modules.toml that names its hash is not an attested recipient.
RUN chown -R root:root /opt/tinycomputer \
 && chmod 755 /opt/tinycomputer \
 && chmod 644 /opt/tinycomputer/libtinycomputer.so /opt/tinycomputer/modules.toml \
 # Fail the build, not the boot, on a shared library the image lacks.
 && missing="$(ldd /usr/local/bin/teeny /opt/tinycomputer/libtinycomputer.so | grep 'not found' || true)" \
 && { [ -z "$missing" ] || { echo "missing shared libraries:" >&2; echo "$missing" >&2; exit 1; }; }

COPY docker/entrypoint.sh /usr/local/bin/teeny-entrypoint
RUN sed -i 's/\r$//' /usr/local/bin/teeny-entrypoint \
 && chmod 755 /usr/local/bin/teeny-entrypoint

# Every directory the volume is mounted over exists in the image, owned by
# the bot, so a fresh named volume starts out with the right owner.
RUN mkdir -p /data/openhuman/home /data/chrome-profile /data/media /data/articles /data/state \
 && chown -R 10001:10001 /data

# The persona and content are read from the working directory at startup.
WORKDIR /app
COPY SOUL.md CONTEXT.md calendar.json ./
COPY skills ./skills
# The serial's bible and voice: the article's rooms read them at boot.
COPY narrative ./narrative
COPY assets ./assets

ENV TEENY_DATA=/data \
    OPENHUMAN_WORKSPACE=/data/openhuman \
    HOME=/data/openhuman/home \
    # The root is read-only; caches go to the /tmp tmpfs.
    XDG_CACHE_HOME=/tmp/.cache \
    BROWSER_CHROME_PATH=/usr/bin/chromium \
    TINYCOMPUTER_MODULE_PATH=/opt/tinycomputer/libtinycomputer.so \
    # The legacy keys, pointed into the volume layout. compose repeats them,
    # because ../.env carries the old relative data/ paths and an env_file
    # beats image ENV.
    MEDIA_OUTPUT_DIR=/data/media \
    REFERENCE_SHEET_PATH=/data/media/reference-sheet.json \
    CHART_OUTPUT_DIR=/data/media/charts \
    MOON_OUTPUT_DIR=/data/media/moon \
    NARRATIVE_LIKENESS_DIR=/data/media/likeness \
    ARTICLES_DIR=/data/articles \
    QUOTE_LOG_PATH=/data/state/quotes.json \
    NARRATIVE_STATE_PATH=/data/state/narrative-state.json \
    BACKROOMS_DIR=/data/state/backrooms

# deploy.sh and cutover.sh tell this image from the legacy one by this label.
LABEL org.opencontainers.image.source="https://github.com/tinyhumansai/teeny" \
      ai.tinyhumans.teeny.runtime="rust"

# Root only until the entrypoint has laid out and handed over the volume; it
# then execs teeny as 10001 through gosu. Use `docker exec -u teeny` to poke
# around, so nothing on the volume ends up root-owned.
# hadolint ignore=DL3002,DL3066
USER root
EXPOSE 3001
HEALTHCHECK --interval=15s --timeout=5s --start-period=30s --retries=5 \
    CMD ["teeny", "healthcheck"]
ENTRYPOINT ["tini", "--", "teeny-entrypoint"]
CMD ["serve"]
