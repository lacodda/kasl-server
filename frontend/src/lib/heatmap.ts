/**
 * Turning a month's cells into a grid of squares: the shape of a month.
 *
 * The arithmetic lives here rather than in the component for the reason the
 * live status does: a cell that quietly falls through to the wrong shade is
 * exactly the kind of defect a screenshot passes over, and a function can be
 * asserted where a `<div>` cannot.
 *
 * The rule the whole view rests on: **a square has several meanings, and only
 * one of them is a number.** Nothing recorded, a day still running, a day away
 * and a finished day are different facts, and a heatmap that paints the first
 * as the palest shade of the last tells a manager somebody worked nothing when
 * nobody said so (ADR 0015). Since 0.28 the squares are drawn from the month's
 * timesheet (ADR 0024), so the shape and the record of a month are one answer.
 */

import type { TimesheetColumn, TimesheetRow, WorkdayKind } from '@/lib/api'

/** What one square of the grid is. */
export type CellKind =
  /** No workday on this date. Not zero hours - no data at all. */
  | 'none'
  /** A day still open on the agent, so it has no total yet. */
  | 'open'
  /** A day the person was away: recorded, but not worked, and not shaded. */
  | 'away'
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
  /** Which kind of day away, for a square that is one. */
  away: WorkdayKind | null
  /**
   * Owed nothing at a full day: a weekend nobody moved, or a holiday. The
   * calendar's fact, never a claim that anybody was off.
   */
  rest: boolean
}

/** The day-of-month number a column is labelled with. */
export function dayOfMonth(date: string): number {
  return Number(date.slice(8, 10))
}

/** Whether a column owes nothing at a full day. */
export function isRest(column: TimesheetColumn): boolean {
  return column.hours === 0
}

/**
 * The busiest finished day of work anywhere in the month: the ceiling every
 * square shares. `null` when nobody finished one.
 *
 * A day away is never the busiest, whatever the agent filed on it: shading by
 * its hours would draw a day of sick leave as the shortest day worked.
 */
export function busiest(rows: TimesheetRow[]): number | null {
  let top: number | null = null
  for (const row of rows) {
    for (const cell of row.days) {
      if (cell.state === 'worked' && cell.worked_seconds !== null) top = Math.max(top ?? 0, cell.worked_seconds)
    }
  }
  return top
}

/**
 * One person's row as squares, one per date of the month.
 *
 * `ceiling` is the grid's shared one, so two rows can be compared with each
 * other. A per-row scale would paint a quiet week and a heavy one identically
 * - each would have its own darkest square - which is the one thing a heatmap
 * is for.
 */
export function squares(row: TimesheetRow, columns: TimesheetColumn[], ceiling: number | null): Square[] {
  return row.days.map((cell, at) => {
    const rest = columns[at] ? isRest(columns[at]) : false
    const base = { date: cell.date, seconds: null, intensity: null, away: null, rest }

    switch (cell.state) {
      case 'worked':
        // Belt and braces on the one field a screen must never guess at: a
        // finished day without a total would otherwise render as a square
        // whose hours are unknown.
        if (cell.worked_seconds === null) return { ...base, kind: 'open' }
        return { ...base, kind: 'worked', seconds: cell.worked_seconds, intensity: scale(cell.worked_seconds, ceiling) }
      case 'open':
        return { ...base, kind: 'open' }
      case 'away':
        return { ...base, kind: 'away', away: cell.kind }
      default:
        return { ...base, kind: 'none' }
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
export function scale(seconds: number, ceiling: number | null): number {
  if (ceiling === null || ceiling <= 0) return 1
  return Math.min(1, Math.max(0.15, seconds / ceiling))
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

/** Whether a string is a month this client can ask for: `YYYY-MM`. */
export function isMonth(value: string | null): value is string {
  return value !== null && /^\d{4}-(0[1-9]|1[0-2])$/.test(value)
}
