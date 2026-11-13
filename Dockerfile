# syntax=docker/dockerfile:1

# 1. Web app
FROM oven/bun:1 AS web
WORKDIR /src/web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ ./
RUN bun run build

# 2. Server, with the web app built in
FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# The build embeds web/dist and checks that web/package.json has the same version.
COPY web/package.json ./web/package.json
COPY --from=web /src/web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p kura \
    && cp target/release/kura /usr/local/bin/kura

# 3. Runtime
FROM debian:bookworm-slim
ARG VERSION=dev
LABEL org.opencontainers.image.title="Kura" \
      org.opencontainers.image.description="Self-hosted downloader for AniWorld and SerienStream" \
      org.opencontainers.image.source="https://github.com/Koxility/kura" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg ca-certificates tini \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 kura \
    && useradd --uid 1000 --gid kura --no-create-home --shell /usr/sbin/nologin kura \
    && mkdir -p /data /media \
    && chown kura:kura /data /media

COPY --from=server /usr/local/bin/kura /usr/local/bin/kura

ENV KURA_LISTEN=0.0.0.0:8787 \
    KURA_DATA=/data \
    KURA_LIBRARY_DIR=/media

USER kura
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8787

HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD ["kura", "healthcheck"]

# tini passes docker stop on to Kura and reaps ffmpeg processes.
ENTRYPOINT ["tini", "--", "kura"]
