/**
 * The month screen's views and what their cells say (ADR 0024).
 *
 * The server decides what each date was; this only decides how it is written
 * on the screen. Kept out of the components for the reason the heatmap's
 * arithmetic is: a cell that quietly says the wrong thing is the defect a
 * screenshot passes over, and a function can be asserted.
 */

import type { Filed, ReportStatus, TimesheetRow } from '@/lib/api'

/**
 * The three ways to read one month, in the order the switch offers them.
 *
 * - `timesheet` - how much: hours and days away, against the norm. What a
 *   company files.
 * - `reports` - whether each day was put a name to, and in time.
 * - `shape` - the pattern: who works weekends, whose month is ragged.
 */
export const VIEWS = ['timesheet', 'reports', 'shape'] as const
export type View = (typeof VIEWS)[number]

/** The view a URL asks for; the timesheet when it asks for none it knows. */
export function viewOf(param: string | null): View {
  return VIEWS.find((view) => view === param) ?? 'timesheet'
}

/**
 * Seconds as the hours a timesheet cell shows: one decimal place, and none
 * when there is nothing after the point - `7.5`, `8`, `0.3`.
 *
 * Tenths rather than minutes because thirty-one columns of `7h 30m` do not
 * fit a screen, and tenths are what a timesheet is filed in. The exact figure
 * is in the cell's label, and the workbook keeps it to the second.
 */
export function hoursShort(seconds: number): string {
  return String(Math.round(seconds / 360) / 10)
}

/** The mark a report cell shows for when the day was filed, as the mockup
 * draws the matrix. */
export const GLYPH: Record<Filed, string> = {
  on_time: '✓',
  late: 'L',
  missing: '✗',
  pending: '·',
}

/**
 * What a manager made of a report: approved and still standing for the day,
 * or something to look at - sent back, or changed since it was reported.
 * `null` when nobody has answered, or there is no report.
 */
export type Standing = 'approved' | 'attention' | null

export function standingOf(report: ReportStatus | null): Standing {
  switch (report) {
    case 'approved':
      return 'approved'
    case 'returned':
    case 'changed':
      return 'attention'
    default:
      return null
  }
}

/**
 * The hours a row's total is measured against.
 *
 * What has come due, not the whole month: on the 10th, a month's norm makes
 * somebody exactly on track read as a third done (ADR 0023). For a month that
 * is over the two are the same figure.
 */
export function measuredAgainst(row: TimesheetRow): number {
  return row.due_seconds
}

/** Days away, all kinds together, for the narrow total column. */
export function daysAway(row: TimesheetRow): number {
  return row.counts.vacation + row.counts.sick + row.counts.day_off
}
