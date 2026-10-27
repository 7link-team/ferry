ARG NODE_VERSION=24.14.0
ARG RUST_VERSION=1.99.0
ARG GO_VERSION=1.27.1
ARG BASE_REGISTRY=docker.io/library

FROM ${BASE_REGISTRY}/node:${NODE_VERSION}-bookworm-slim AS web
WORKDIR /build
COPY package.json package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY index.html tsconfig.json vite.config.ts ./
COPY src ./src
RUN npm run build

FROM ${BASE_REGISTRY}/golang:${GO_VERSION}-bookworm AS actions
WORKDIR /build
COPY workers/actions ./workers/actions
COPY scripts/build-actions-workers.sh ./scripts/build-actions-workers.sh
RUN --mount=type=cache,target=/go/pkg/mod \
    --mount=type=cache,target=/root/.cache/go-build \
    bash scripts/build-actions-workers.sh /out/actions-workers

FROM ${BASE_REGISTRY}/rust:${RUST_VERSION}-bookworm AS server
WORKDIR /build
ARG BUILD_JOBS=2
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY --from=web /build/dist ./dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo test --locked --jobs ${BUILD_JOBS} && \
    cargo build --locked --release --jobs ${BUILD_JOBS} -p ferry-server -p ferry-runner && \
    install -D target/release/ferry-server /out/ferry-server && \
    install -D target/release/ferry-runner /out/ferry-runner

# The runner binaries the host-enrollment command downloads.
FROM scratch AS artifacts
COPY --from=server /out/ferry-server /linux/ferry-server
COPY --from=server /out/ferry-runner /linux/ferry-runner

FROM ${BASE_REGISTRY}/debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 ferry \
    && useradd --uid 10001 --gid ferry --no-create-home ferry \
    && mkdir /data && chown ferry:ferry /data
COPY --from=server /out/ferry-server /usr/local/bin/ferry-server
COPY --from=actions /out/actions-workers /usr/local/bin/actions-workers
COPY --from=server /out/ferry-runner /usr/local/share/ferry/runner/linux-amd64/ferry-runner
ENV FERRY_DATA_DIR=/data FERRY_BIND=0.0.0.0:4290 FERRY_SERVER_MODE=true
USER 10001:10001
VOLUME ["/data"]
EXPOSE 4290
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:4290/api/status || exit 1
ENTRYPOINT ["/usr/local/bin/ferry-server"]
