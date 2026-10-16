# syntax=docker/dockerfile:1.7
# Platter Bridge: Platter's home side without a window (docs/BRIDGE.md).
# Build from the repository root:
#   docker build -f docker/bridge/Dockerfile -t platter-bridge .

FROM rust:1-bookworm AS build
# The crate is the desktop app's too, so its build scripts look for the
# desktop libraries. The Bridge binary links none of them (checked below).
RUN apt-get update \
 && apt-get install -y --no-install-recommends libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev libasound2-dev libssl-dev pkg-config \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
# The desktop's pages aren't part of the Bridge; the build only needs the folder.
RUN mkdir -p dist && printf '<!doctype html>\n' > dist/index.html
ENV RUSTFLAGS="-C link-arg=-Wl,--as-needed"
ARG TARGETPLATFORM
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry-${TARGETPLATFORM} \
    --mount=type=cache,target=/src/src-tauri/target,id=bridge-target-${TARGETPLATFORM} \
    cd src-tauri \
 && cargo build --locked --profile bridge --bin platter-bridge \
 && cp target/bridge/platter-bridge /platter-bridge
# Nothing of the desktop (webview, audio) may be linked in.
RUN ldd /platter-bridge && ! ldd /platter-bridge | grep -Eiq 'webkit|gtk|asound|appindicator'

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tzdata \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1000 --user-group --home-dir /data --shell /usr/sbin/nologin platter \
 && mkdir -p /data \
 && chown platter:platter /data
COPY --from=build /platter-bridge /usr/local/bin/platter-bridge
LABEL org.opencontainers.image.title="Platter Bridge" \
      org.opencontainers.image.description="Optional home side for Platter: Lidarr catch-up while no computer has Platter open." \
      org.opencontainers.image.source="https://github.com/Silvercyclops999/platter" \
      org.opencontainers.image.licenses="GPL-3.0-or-later"
ENV BRIDGE_DATA=/data
VOLUME /data
USER 1000:1000
# /health listens on 127.0.0.1 inside the container only; nothing is published.
HEALTHCHECK --interval=60s --timeout=10s --start-period=60s --retries=3 CMD ["/usr/local/bin/platter-bridge", "health"]
ENTRYPOINT ["/usr/local/bin/platter-bridge"]
