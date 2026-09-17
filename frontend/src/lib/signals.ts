/**
 * Turning a signal into words and a chart into bars.
 *
 * Kept out of the components for the reason `statusTone` and the heatmap's
 * shading are: what a signal *says* is the part that can be wrong in a way a
 * screenshot does not reveal, and a function can be asserted where a `<div>`
 * cannot.
 *
 * The rule the wording follows, from ADR 0016: **a signal is a question, not a
 * verdict.** Every phrase here states what the server measured - hours, weeks,
 * days - and none of them says what it means. Somebody's hours falling for
 * three weeks is a holiday, a hospital, or a project that ended, and this
 * screen is not entitled to guess which.
 */

import type { Signal, SignalKind, TrendWeek } from '@/lib/api'

/**
 * How a signal is drawn. Never `bad`: none of these is a finding against
 * anybody, and painting a person's row red would make the screen an accusation
 * rather than a place to start a conversation.
 */
export type SignalTone = 'warn' | 'info'

export function signalTone(kind: SignalKind): SignalTone {
  switch (kind) {
    case 'no_data':
      // The only one that is reliably a problem *with the installation* - an
      // agent that stopped reporting makes every other number about that
      // person untrustworthy too.
      return 'warn'
    case 'declining':
    case 'unusual_week':
      return 'info'
  }
}

/**
 * The i18n key and interpolation values for a signal's sentence.
 *
 * Returned rather than rendered so the values can be checked without a
 * translation table: the defect worth catching is a signal quoting the wrong
 * figure, not a missing string.
 */
export function signalPhrase(signal: Signal): { key: string; values: Record<string, unknown> } {
  switch (signal.kind) {
    case 'declining':
      return {
        key: 'signals.declining',
        values: {
          weeks: signal.weeks ?? 0,
          from: hours(signal.from_seconds),
          to: hours(signal.to_seconds),
        },
      }
    case 'no_data':
      return { key: 'signals.noData', values: { days: signal.days_quiet ?? 0 } }
    case 'unusual_week':
      return {
        key: signal.to_seconds !== null && signal.median_seconds !== null && signal.to_seconds > signal.median_seconds
          ? 'signals.unusualHigh'
          : 'signals.unusualLow',
        values: { week: hours(signal.to_seconds), median: hours(signal.median_seconds) },
      }
  }
}

/**
 * Seconds as a bare number of hours: `8.5`, `40`.
 *
 * One decimal, and no unit - the unit belongs to the sentence, which is
 * translated. `—` for a figure the server did not send, so a missing value can
 * never render as a confident zero.
 */
export function hours(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return '—'
  const value = seconds / 3600
  // A whole number of hours reads better without the trailing zero.
  return Number.isInteger(value) ? String(value) : value.toFixed(1)
}

/** One bar on the trend chart. */
export interface Bar {
  week_start: string
  /** The week's hours, or `null` for a week with no days recorded at all.
   *
   * The distinction is the whole point and it is not a height: a week of
   * nothing and a week of twenty minutes are different facts, and the chart
   * draws the first as a gap rather than as a very short bar. How tall the
   * rest are is the chart's arithmetic, not this function's. */
  worked_seconds: number | null
}

/**
 * The weeks, as the chart takes them.
 *
 * An empty week keeps its place and says so with `null`: a chart that dropped
 * it would close the gap up and turn an absence into continuity, which is the
 * one thing the trend exists to show.
 */
export function bars(weeks: TrendWeek[]): Bar[] {
  return weeks.map((week) => ({
    week_start: week.week_start,
    worked_seconds: week.days_recorded === 0 ? null : week.worked_seconds,
  }))
}

/** `Aug 24` - the label under a bar, in the reader's language. */
export function weekLabel(weekStart: string, locale: string): string {
  // Midday, so no zone shifts the label onto a neighbouring day.
  return new Date(`${weekStart}T12:00:00`).toLocaleDateString(locale, { month: 'short', day: 'numeric' })
}
