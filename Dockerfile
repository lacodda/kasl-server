# Build and run kasl-server in a container.
#
# Built on the target machine (the stand is a Raspberry Pi, so aarch64), which
# also makes this the one place where a build for that architecture is proven
# on every deploy rather than only at release time.

# The web UI first: it is compiled into the binary (ADR 0012), so the Rust
# build needs `frontend/dist` to already hold the real thing rather than the
# placeholder the repository carries.
FROM node:22-slim AS web
WORKDIR /web
RUN corepack enable
# Manifest and lockfile alone, so a source edit does not re-resolve the tree.
COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY frontend/ ./
RUN pnpm build

FROM rust:1-slim-trixie AS build
WORKDIR /src

# Dependencies first, so editing source does not re-download and rebuild the
# whole tree. A stub main gives cargo something to compile them against.
COPY Cargo.toml Cargo.lock ./
# rust-embed reads this directory at compile time, so it has to exist even for
# the dependency-only layer.
RUN mkdir -p src frontend/dist \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && cargo build --release \
    && rm -rf src

COPY src ./src
COPY migrations ./migrations
COPY --from=web /web/dist ./frontend/dist
# Touch the entry points: cargo skips a rebuild when timestamps look older
# than the artifacts left by the dependency layer.
RUN touch src/main.rs src/lib.rs && cargo build --release

FROM debian:trixie-slim
# ca-certificates for TLS to PostgreSQL; curl so the healthcheck below needs no
# extra layer of its own.
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# An unprivileged user: the server needs nothing that root grants.
RUN useradd --system --create-home --uid 10001 kasl
USER kasl

COPY --from=build /src/target/release/kasl-server /usr/local/bin/kasl-server

EXPOSE 8080
# Readiness, not just liveness: /health round-trips to the database, so a
# container that answers has actually reached its storage.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["kasl-server"]
