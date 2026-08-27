# 0012. Serving the web UI: embedded in the binary, and a kit that matches kilna's

Date: 2026-08-26

Status: Accepted

Amends [ADR 0002](0002-frontend-stack.md), whose component-kit decision no longer
describes the stack it was pointing at.

## Context

Every endpoint through 0.10.0 answers JSON, and nothing serves a page. The web
UI arrives now, which forces two questions that outlive this milestone: how the
built SPA reaches a browser, and what the components are made of.

**ADR 0002 has drifted.** It decided in August that both products would keep "a
small hand-rolled `ui/` kit, no shadcn/radix". kilna has since moved to Radix
primitives with `class-variance-authority` and `tailwind-merge`, and its kit is
fourteen components under `src/components/ui/`. Following ADR 0002 literally
would mean writing a second, different kit - the exact duplication ADR 0002
existed to prevent.

## Decision

**The built SPA is embedded in the server binary.** `rust-embed` compiles
`frontend/dist` into the executable; a fallback route serves `index.html` for
any path the API does not claim, so client-side routes survive a refresh.

A single file is the whole product for a self-hosted install: no web server to
configure, no directory to keep in sync with the binary, no way to deploy a
frontend built from a different commit than the API it calls. The alternative -
serving from a directory with `tower-http`'s `ServeDir` - is easier to develop
against but makes "which build is this" a question about the filesystem instead
of about the version.

Development does not pay for this. Vite serves the app on its own port and
proxies `/api` to the server, so a rebuild is a page reload rather than a Rust
compile. The embed is what `cargo build --release` produces.

**The kit follows kilna as it is now, not as ADR 0002 described it.** Radix
primitives, `class-variance-authority` for variants, `tailwind-merge` behind a
`cn` helper, components under `src/components/ui/` - the same layout, so a
component moves between the products by copying it and swapping tokens.

Radix earns its place on the parts that are hard to get right and invisible when
wrong: focus traps in dialogs, keyboard behaviour in menus and selects, the
accessible names a screen reader reads. A hand-rolled kit does not skip that
work, it postpones it.

**Semantic tokens, one accent.** The token vocabulary is the mockup's
(`bg` / `raise` / `line` / `dim` / `accent`), not stock shadcn names, so a screen
can be checked against the mockup in the mockup's own words. The accent is the
product's gold, `#D9A82E`, darkened to `#8A6A18` for text on light ground.
Components never use `dark:` utilities - every colour goes through a token, and
the theme swaps the tokens.

**Sessions stay as they are.** The SPA authenticates with the same cookie ADR
0007 defined, so the browser and an API client are the same path. No token in
`localStorage`, which is what a stateless scheme would have forced.

## Consequences

`frontend/` sets a second toolchain: Node and pnpm join the stage-start ritual,
and CI builds the SPA before the crate that embeds it. A release now fails if
the frontend does not compile, which is correct - a binary carrying a stale
`dist` would be worse.

`rust-embed` needs `frontend/dist` to exist at compile time. A checked-in
placeholder keeps `cargo build` working in a clean checkout without Node, and
the real `dist` replaces it in CI and in the Docker build.

ADR 0002's other half stands: the same React/TypeScript/Vite/Tailwind/i18next
stack, and extraction into a shared package still waits for the components to
prove themselves in both products rather than being written for reuse first.
