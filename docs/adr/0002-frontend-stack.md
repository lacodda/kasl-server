# 0002. Frontend stack: shared with kilna

Date: 2026-08-12
Status: Accepted, component-kit decision amended by [ADR 0012](0012-serving-the-web-ui.md)

## Context

The web UI (manager dashboard, employee personal page, admin) needs a frontend stack. kilna — another product of the line — already ships a working one: React 19 + TypeScript + Vite + Tailwind CSS 4 (`@theme` tokens) + i18next, with a small hand-rolled `ui/` kit (Button, Input, Select) instead of a component library. Divergent stacks would mean every shared component is written twice.

## Decision

- Adopt kilna's stack unchanged: **React 19, TypeScript, Vite, Tailwind CSS 4, i18next**, ESLint config included.
- Keep the same mini-kit approach: no shadcn/radix; small owned components styled with Tailwind. The product token family is `--color-ks-*` (gold `#D9A82E`; darkened for interactive text on light ground).
- Components proven useful to both products (Card, StatTile, Badge, Table, Tabs) are written kit-style so they can be lifted into a shared line package — extraction happens when a second consumer actually exists, not preemptively.
- Product-specific visualizations (day timeline, hour bars, trend chart) stay in this repo.

## Consequences

- Fixes learned in one product's kit transfer to the other by copy with a token swap, later by a shared package.
- The UI mockup (design reference for the v0.4.0–v0.6.0 milestones) defines the visual language: one gold accent, separate status semantics (never color alone), mono for figures, both themes token-driven.
- Tailwind 4 and React 19 set the minimum toolchain for `frontend/`; the stage-start ritual updates both products in step.
