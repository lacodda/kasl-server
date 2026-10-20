/**
 * Reading the read tokens, and saying where a dashboard should point.
 *
 * Out of the component because these are the parts that can be wrong in a way
 * a screenshot hides: a revoked token drawn as live reads perfectly and is a
 * lie, and a URL with the range variables misspelled pastes into Grafana and
 * answers 400 on every panel.
 */

import type { ReadToken } from '@/lib/api'

/** The datasets of `/api/bi/v1`, in the order the reference lists them. */
export const DATASETS = ['people', 'departments', 'days', 'summary'] as const
export type Dataset = (typeof DATASETS)[number]

/** The two that answer a range, and so need one in their URL. */
const RANGED: ReadonlySet<Dataset> = new Set(['days', 'summary'])

/**
 * Where a dataset is, for a tool that fills in its own range.
 *
 * The range is Grafana's dashboard time picker, in the form the server reads:
 * a tool that sends its own dates replaces the two variables. Built from the
 * page's own origin, because that is the address the reader reached the
 * server at - the one thing the server cannot know about itself without
 * `KASL_PUBLIC_URL`.
 */
export function datasetUrl(origin: string, dataset: Dataset, format: 'json' | 'csv' = 'json'): string {
  const path = `${origin.replace(/\/+$/, '')}/api/bi/v1/${dataset}${format === 'csv' ? '.csv' : ''}`
  return RANGED.has(dataset) ? `${path}?from=\${__from:date:YYYY-MM-DD}&to=\${__to:date:YYYY-MM-DD}` : path
}

/**
 * Where one token stands.
 *
 * - `revoked` - reads nothing, and never will again.
 * - `unused` - live, and nothing has read with it yet: issued and not yet
 *   pasted anywhere, or pasted somewhere that never asked.
 * - `live` - live, and read with.
 */
export type TokenState = 'live' | 'unused' | 'revoked'

export function tokenState(token: ReadToken): TokenState {
  // Revoked first: a token used every minute until it was revoked is revoked.
  if (token.revoked_at) return 'revoked'
  return token.last_used_at ? 'live' : 'unused'
}
