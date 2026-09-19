/**
 * Turning an alert into words.
 *
 * Kept out of the component for the reason `signalPhrase` is: what an alert
 * *says* is the part that can be wrong in a way a screenshot does not reveal -
 * a sentence quoting the threshold where it meant the measurement reads
 * perfectly and is a lie - and a function can be asserted where a `<div>`
 * cannot.
 *
 * The rule the wording follows is the one the signals follow, and for the same
 * reason: **every phrase states what was measured and what it was measured
 * against, and none of them says what it means.** A day that ran to twelve
 * hours is a release, a crisis, or somebody who forgot to close kasl, and the
 * server knows none of that. The difference from a signal is only that this
 * one did not wait to be asked.
 */

import type { Alert, AlertRule } from '@/lib/api'

/**
 * How an alert is drawn. Never `bad`: none of these is a finding against
 * anybody. Silence is `warn` because it is the one that is reliably a problem
 * with the *installation* - an agent that stopped reporting makes every other
 * number about that person untrustworthy too - and the other two are `info`,
 * which is what a question looks like.
 */
export type AlertTone = 'warn' | 'info'

export function alertTone(rule: AlertRule): AlertTone {
  switch (rule) {
    case 'no_agent_data':
      return 'warn'
    case 'overwork':
    case 'day_not_closed':
      return 'info'
  }
}

/**
 * The i18n key and interpolation values for an alert's sentence.
 *
 * Returned rather than rendered so the values can be checked without a
 * translation table: the defect worth catching is an alert quoting the wrong
 * figure, not a missing string.
 */
export function alertPhrase(alert: Alert): { key: string; values: Record<string, unknown> } {
  switch (alert.rule) {
    case 'no_agent_data':
      return { key: 'alerts.noAgentData', values: { span: span(alert.observed_seconds) } }
    case 'overwork':
      return {
        key: 'alerts.overwork',
        values: { worked: hours(alert.observed_seconds), norm: hours(alert.against_seconds), date: alert.subject_date ?? '' },
      }
    case 'day_not_closed':
      return {
        key: 'alerts.dayNotClosed',
        values: { span: span(alert.observed_seconds), date: alert.subject_date ?? '' },
      }
  }
}

/**
 * Seconds as a bare number of hours: `8.5`, `40`.
 *
 * The same shape the signals use, deliberately: the two sit on one screen, and
 * "12.5 h" beside "12 h 30 m" reads as two different measurements of two
 * different things. `—` for a figure the server did not send, so a missing
 * value can never render as a confident zero.
 */
export function hours(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return '—'
  const value = seconds / 3600
  return Number.isInteger(value) ? String(value) : value.toFixed(1)
}

/**
 * A stretch of time as a reader would say it: `9 h`, `2 d`, `13 mo`.
 *
 * Used where the figure is elapsed time rather than hours worked - silence,
 * and how long a day has been open - because those have no upper bound. A
 * real stand carried a day left open since the previous August, and the
 * honest reading of it in hours was `9460.7 h`, which is a number nobody
 * converts in their head. Hours are right up to a couple of days and wrong
 * after that.
 *
 * Deliberately **not** used for the overwork figures. Those sit next to a
 * norm in the same sentence - "worked 12.6 h against a norm of 8 h" - and two
 * quantities being compared have to carry the same unit, or the comparison is
 * the reader's job instead of the sentence's.
 *
 * One unit and no remainder: "2 d" rather than "2 d 4 h". The alert exists to
 * say a threshold is well past, and the second unit is precision about
 * something nobody is going to act on to the hour.
 */
export function span(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return '—'

  const HOUR = 3600
  const DAY = 24 * HOUR
  // A month of thirty days and a year of 365: these are labels on a bar that
  // has already been cleared, not arithmetic anybody balances a timesheet
  // with. What matters is that every span lands in exactly one bucket - steps
  // built on different divisors leave gaps where a value belongs to neither.
  const MONTH = 30 * DAY
  const YEAR = 365 * DAY

  if (seconds >= YEAR) return `${Math.floor(seconds / YEAR)} y`
  if (seconds >= MONTH) return `${Math.floor(seconds / MONTH)} mo`
  if (seconds >= 2 * DAY) return `${Math.floor(seconds / DAY)} d`
  return `${hours(seconds)} h`
}

/**
 * Whether an alert is still waiting for somebody.
 *
 * A single predicate rather than `=== 'open'` spelled out at each call site:
 * the feed, the badge and the row's own controls all have to agree about what
 * "unattended" means, and three copies of one comparison is three chances for
 * one of them to disagree after a fourth state is added.
 */
export function isOpen(alert: Alert): boolean {
  return alert.state === 'open'
}
