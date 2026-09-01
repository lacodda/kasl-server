/**
 * Turning a month's cells into a grid.
 *
 * The arithmetic lives here rather than in the component for the reason the
 * live status does: a cell that quietly falls through to the wrong shade is
 * exactly the kind of defect a screenshot passes over, and a function can be
 * asserted where a `<div>` cannot.
 *
 * The rule the whole screen rests on: **a square has four meanings, and only
 * one of them is a number.** Nothing recorded, a day still running, a finished
 * day, and a date outside the person's data are four different facts, and a
 * heatmap that paints the first as the palest shade of the third tells a
 * manager somebody worked nothing when nobody said so.
 */

import type { HeatmapCell, HeatmapRow } from '@/lib/api'

/** What one square of the grid is. */
export type CellKind =
  /** No workday on this date. Not zero hours - no data at all. */
  | 'none'
  /** A day still open on the agent, so it has no total yet. */
  | 'open'
  /** A finished day, with hours behind it. */
  | 'worked'

export interface Square {
  date: string
  kind: CellKind
  /** Seconds worked; `null` for anything but a finished day. */
  seconds: number | null
  /**
   * How full this day is against the grid's busiest, 0 to 1. `null` where
   * there is no figure to place on the scale.
   */
  intensity: number | null
  /** Saturday or Sunday, from the date itself. */
  weekend: boolean
}

/** Every date of a month, as `YYYY-MM-DD`. */
export function monthDates(from: string, to: string): string[] {
  const dates: string[] = []
  // Stepped as text through a UTC date so no local zone can shift a day: a
  // date here is a label, not a moment (ADR 0003).
  const cursor = new Date(`${from}T00:00:00Z`)
  const last = new Date(`${to}T00:00:00Z`)
  while (cursor <= last) {
    dates.push(cursor.toISOString().slice(0, 10))
    cursor.setUTCDate(cursor.getUTCDate() + 1)
  }
  return dates
}

/** Whether a `YYYY-MM-DD` date falls on a Saturday or a Sunday. */
export function isWeekend(date: string): boolean {
  // Deliberately only the weekend, never "day off": which days a company
  // actually rests is a calendar this server does not have until v0.21, and
  // guessing it would mark someone's ordinary Saturday shift as unusual.
  const weekday = new Date(`${date}T00:00:00Z`).getUTCDay()
  return weekday === 0 || weekday === 6
}

/** The day-of-month number a column is labelled with. */
export function dayOfMonth(date: string): number {
  return Number(date.slice(8, 10))
}

/**
 * One person's row as squares, one per date of the month.
 *
 * `busiest` is the grid's shared ceiling, so two rows can be compared with
 * each other. A per-row scale would paint a quiet week and a heavy one
 * identically - each would have its own darkest square - which is the one
 * thing a heatmap is for.
 */
export function squares(row: HeatmapRow, dates: string[], busiest: number | null): Square[] {
  const byDate = new Map<string, HeatmapCell>(row.days.map((cell) => [cell.date, cell]))

  return dates.map((date) => {
    const cell = byDate.get(date)
    const weekend = isWeekend(date)

    if (!cell) return { date, kind: 'none', seconds: null, intensity: null, weekend }
    if (cell.open || cell.worked_seconds === null) {
      return { date, kind: 'open', seconds: null, intensity: null, weekend }
    }

    return {
      date,
      kind: 'worked',
      seconds: cell.worked_seconds,
      intensity: scale(cell.worked_seconds, busiest),
      weekend,
    }
  })
}

/**
 * Where a day sits on the scale, 0 to 1.
 *
 * A floor of 0.15 rather than 0: a twenty-minute day is a real day, and a
 * square indistinguishable from an empty one would file it under "no data" -
 * the confusion this whole module exists to prevent.
 */
export function scale(seconds: number, busiest: number | null): number {
  if (busiest === null || busiest <= 0) return 1
  return Math.min(1, Math.max(0.15, seconds / busiest))
}

/**
 * The steps a square's fill is snapped to.
 *
 * Discrete rather than continuous: five shades can be told apart at a glance
 * and matched to a legend, where a smooth gradient only says "more" and "less"
 * and cannot be read back to a number.
 */
export const STEPS = [0.2, 0.4, 0.6, 0.8, 1] as const

/** Which step an intensity lands on, 1 to 5. */
export function step(intensity: number): number {
  const index = STEPS.findIndex((edge) => intensity <= edge)
  return index === -1 ? STEPS.length : index + 1
}

/** The month before or after a `YYYY-MM`, as `YYYY-MM`. */
export function shiftMonth(month: string, by: number): string {
  const year = Number(month.slice(0, 4))
  const index = Number(month.slice(5, 7))
  // Month 0 is January here, so `index - 1` puts it in range and the
  // constructor rolls a 13th month into the next year on its own.
  const moved = new Date(Date.UTC(year, index - 1 + by, 1))
  return `${moved.getUTCFullYear()}-${String(moved.getUTCMonth() + 1).padStart(2, '0')}`
}

/** The current month as `YYYY-MM`, in the reader's own zone. */
export function currentMonth(now: Date = new Date()): string {
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}`
}
