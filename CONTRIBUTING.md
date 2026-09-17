# Contributing to kasl-server

## Working on the web UI

```console
$ pnpm --dir frontend install
$ pnpm --dir frontend dev      # http://localhost:5173, proxies /api to :8080
```

Vite serves the app and proxies `/api` to the server, so the browser sees one
origin and the session cookie behaves as it will in production. A change is a
page reload rather than a Rust compile.

```console
$ pnpm --dir frontend lint     # eslint, tsc and the unit tests
$ pnpm --dir frontend build    # what gets embedded
```

**TypeScript is held at 6.x on purpose.** typescript-eslint does not run under
TS 7 yet, so upgrading takes linting with it - `pnpm update --latest` pulls 7
and `pnpm lint` then fails before it type-checks anything. The `^6.0.3` range
in `frontend/package.json` is what keeps that from happening by accident.

`cargo build --release` embeds whatever is in `frontend/dist` at that moment.
The repository carries an empty placeholder there, so a checkout without Node
still compiles - and a binary built that way says
`no web UI was built into this binary` instead of serving a blank page. The
decision and its trade-offs are in
[ADR 0012](https://github.com/lacodda/kasl-server/blob/main/docs/adr/0012-serving-the-web-ui.md).

## The gate

```console
$ cargo fmt --check
$ cargo clippy --all-targets -- -D warnings
$ cargo test
$ pnpm --dir frontend lint
$ pnpm --dir frontend build
```

This is what CI runs, split across a Rust matrix (Linux and Windows), a
PostgreSQL-backed database job, a frontend job, and an MSRV build against the
`rust-version` declared in `Cargo.toml`. A green terminal here means a green
pull request.

## Architecture decisions

Anything that would be asked about again later is written down in
[`docs/adr/`](https://github.com/lacodda/kasl-server/tree/main/docs/adr) as a
short Context / Decision / Consequences note, in the same commit as the
change.

## Stack

Rust REST API (axum) + PostgreSQL (sqlx); the web UI is React 19 + TypeScript
+ Vite + Tailwind 4, built into the binary.

## Commits

Conventional Commits, English, no trailers.
