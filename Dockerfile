# syntax=docker/dockerfile:1
# mise: the API, the web app and the admin CLI in one image.

FROM node:24-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# The same Rust as rust-toolchain.toml.
FROM rust:1.99.1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p mise-api -p mise-cli \
    && cp target/release/mise-server target/release/mise /usr/local/bin/

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --home-dir /app --shell /usr/sbin/nologin mise \
    && mkdir -p /data/media && chown mise:mise /data/media
COPY --from=server /usr/local/bin/mise-server /usr/local/bin/mise /usr/local/bin/
COPY --from=web /src/web/dist /app/web
ENV MISE_BIND=0.0.0.0:8080 \
    MISE_WEB_DIR=/app/web \
    MEDIA_ROOT=/data/media \
    RUST_LOG=info,sqlx=error,tower_http=warn
USER mise
WORKDIR /app
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1
CMD ["mise-server"]
