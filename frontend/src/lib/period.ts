/**
 * Periods: a day, a week or a month, and the arithmetic of moving between
 * them and comparing one with the one before (ADR 0023).
 *
 * The server has always taken a range; a period is a range with a name. The
 * unit lives here and in the URL, never in the API: `/team/days` answers
 * whatever range it is asked, and a screen that pages by month asks for a
 * month.
 *
 * Dates stay text, as everywhere in this client (see `day.ts`): a calendar
 * date is a label, and it only becomes a `Date` for a moment at local midday
 * while a neighbour is worked out.
 */

import { isoDate } from '@/lib/day'

export type PeriodUnit = 'day' | 'week' | 'month'

/** The units, in the order the switch offers them. */
export const PERIOD_UNITS: readonly PeriodUnit[] = ['day', 'week', 'month']

/** A period: its unit and its first and last dates, both inclusive. */
export interface Period {
  unit: PeriodUnit
  from: string
  to: string
}

/** A `YYYY-MM-DD` as a local date at midday, safe from zone shifts. */
function local(date: string): Date {
  const [year = 0, month = 1, day = 1] = date.split('-').map(Number)
  return new Date(year, month - 1, day, 12)
}

/** Whether a string is a real calendar date, not just shaped like one. */
function isDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false
  return isoDate(local(value)) === value
}

/** The period of `unit` that contains `date`. */
export function periodOf(unit: PeriodUnit, date: string): Period {
  const at = local(date)
  switch (unit) {
    case 'day':
      return { unit, from: date, to: date }
    case 'week': {
      // getDay() is 0 on Sunday, which belongs to the week that started six
      // days earlier.
      const monday = new Date(at.getFullYear(), at.getMonth(), at.getDate() - ((at.getDay() + 6) % 7), 12)
      const sunday = new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + 6, 12)
      return { unit, from: isoDate(monday), to: isoDate(sunday) }
    }
    case 'month': {
      const first = new Date(at.getFullYear(), at.getMonth(), 1, 12)
      // Day zero of the next month is the last day of this one.
      const last = new Date(at.getFullYear(), at.getMonth() + 1, 0, 12)
      return { unit, from: isoDate(first), to: isoDate(last) }
    }
  }
}

/** The period `steps` units away: -1 is the one before, 1 the one after. */
export function shiftPeriod(period: Period, steps: number): Period {
  const at = local(period.from)
  switch (period.unit) {
    case 'day':
      return periodOf('day', isoDate(new Date(at.getFullYear(), at.getMonth(), at.getDate() + steps, 12)))
    case 'week':
      return periodOf('week', isoDate(new Date(at.getFullYear(), at.getMonth(), at.getDate() + steps * 7, 12)))
    case 'month':
      // From the first of the month, so the 31st never overflows into the
      // month after the one meant.
      return periodOf('month', isoDate(new Date(at.getFullYear(), at.getMonth() + steps, 1, 12)))
  }
}

/**
 * The part of a period that has happened: its start to today, or all of it
 * once it is over. `null` for a period that has not begun.
 *
 * What a screen asks the server for, and what it exports. On the 10th, "this
 * month" measured against the whole month's norm reads as 30% done for
 * somebody exactly on track; against the norm due so far it reads as 100%,
 * which is the answer to "am I on track" (ADR 0023). Dates compare as text:
 * `YYYY-MM-DD` sorts as it counts.
 */
export function elapsed(period: Period, today: string): { from: string; to: string } | null {
  if (period.from > today) return null
  return { from: period.from, to: period.to < today ? period.to : today }
}

/** Every date of a period, in order. */
export function periodDates(period: Period): string[] {
  const dates: string[] = []
  const end = local(period.to)
  for (let at = local(period.from); at <= end; at = new Date(at.getFullYear(), at.getMonth(), at.getDate() + 1, 12)) {
    dates.push(isoDate(at))
  }
  return dates
}

/**
 * The period a URL asks for: `?period=month&from=2026-09-01`.
 *
 * Any date inside the period will do as `from` - it is normalized to the
 * period's first day. Anything missing or malformed falls back rather than
 * failing: the unit to `fallbackUnit`, the date to `today`. `date` - the day a
 * notice links to - anchors the period when `from` is absent, so a link to one
 * day opens the period around it.
 */
export function periodFromParams(params: URLSearchParams, today: string, fallbackUnit: PeriodUnit = 'week'): Period {
  const asked = params.get('period')
  const unit = PERIOD_UNITS.includes(asked as PeriodUnit) ? (asked as PeriodUnit) : fallbackUnit
  const anchor = [params.get('from'), params.get('date')].find((value): value is string => value !== null && isDate(value))
  return periodOf(unit, anchor ?? today)
}

/** A period as the URL parameters `periodFromParams` reads back. */
export function periodParams(period: Period): Record<string, string> {
  return { period: period.unit, from: period.from }
}

/**
 * The share of their norm a person worked, or `null` where the period owed
 * nothing - a Sunday, a fortnight of leave. Null rather than infinite: a share
 * of nothing is not a very large number, it is not a number.
 */
export function shareOfNorm(workedSeconds: number, normSeconds: number): number | null {
  return normSeconds > 0 ? workedSeconds / normSeconds : null
}

/**
 * How a share moved against the period before, in percentage points, or
 * `null` when either side has no share to compare.
 *
 * The share rather than the hours, because September has more working days
 * than August: comparing their raw hours compares calendars (ADR 0023).
 */
export function shareChange(current: number | null, previous: number | null): number | null {
  if (current === null || previous === null) return null
  return Math.round((current - previous) * 100)
}

/** A share as a whole percentage: `0.875` is `88%`. */
export function percent(share: number): string {
  return `${Math.round(share * 100)}%`
}
