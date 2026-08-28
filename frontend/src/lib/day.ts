/**
 * Turning stored days into what the timeline draws.
 *
 * Two rules run through all of it:
 *
 * * **A date is a label, not a moment.** The server stores the employee's own
 *   calendar date (ADR 0003). Passing `2026-08-24` through `new Date()` reads
 *   it as UTC midnight, which in a western zone is the 23rd - the day would be
 *   filed under yesterday for exactly the people the offset rules exist for.
 *   So dates are handled as text and only timestamps become `Date`s.
 * * **Nothing here invents a figure.** Hours come from the server, which
 *   computed them from what it stored; a second calculation in the browser
 *   would disagree with the manager's dashboard sooner or later.
 */

import i18n from '@/i18n'
import type { Day } from '@/lib/api'

/** `YYYY-MM-DD` for a date, in the browser's own zone rather than UTC. */
export function isoDate(date: Date): string {
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  return `${year}-${month}-${day}`
}

/** The Monday of the week containing `date`, as a local date. */
export function startOfWeek(date: Date): Date {
  const monday = new Date(date.getFullYear(), date.getMonth(), date.getDate())
  // getDay() is 0 on Sunday, which belongs to the week that started six days
  // earlier rather than the one about to start.
  const weekday = (monday.getDay() + 6) % 7
  monday.setDate(monday.getDate() - weekday)
  return monday
}

/** A week: seven dates, Monday first. A tuple, so its ends are always there. */
export type Week = [string, string, string, string, string, string, string]

/** The seven local dates of the week that `monday` starts, as `YYYY-MM-DD`. */
export function weekDates(monday: Date): Week {
  const dates = Array.from({ length: 7 }, (_, offset) => {
    const date = new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + offset)
    return isoDate(date)
  })
  return dates as Week
}

/** Moves a week boundary by whole weeks, forward or back. */
export function shiftWeeks(monday: Date, weeks: number): Date {
  return new Date(monday.getFullYear(), monday.getMonth(), monday.getDate() + weeks * 7)
}

/**
 * Hours and minutes as `7h 30m`, or `—` for a figure the server does not have.
 *
 * An open day has no total, and `0h 0m` would be a claim rather than a gap.
 */
export function duration(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return '—'
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.round((seconds % 3600) / 60)
  // Rounding 59m30s up must not produce "7h 60m".
  if (minutes === 60) return `${hours + 1}h 0m`
  return `${hours}h ${minutes}m`
}

/** A timestamp as `09:12` in the reader's own zone. */
export function clock(timestamp: string): string {
  return new Date(timestamp).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit', hour12: false })
}

/** The short weekday of a `YYYY-MM-DD` date - `Mon`, and not in another tongue. */
export function weekdayName(date: string): string {
  // Midday, so no zone shifts the label onto a neighbouring day.
  return new Date(`${date}T12:00:00`).toLocaleDateString(locale(), { weekday: 'short' })
}

/**
 * The locale for dates and times: the app's language, not the machine's.
 *
 * `toLocaleDateString([])` follows the operating system, which put Russian
 * weekday names in an otherwise English page - found by looking at the screen,
 * not by a test. Formatting follows the language the UI is shown in, so it
 * moves when the interface is translated and not before.
 */
function locale(): string {
  return i18n.language || 'en'
}

/** One band on the timeline, as a percentage of the day's span. */
export interface Band {
  /** Distance from the day's start, 0-100. */
  left: number
  /** Width, 0-100. Never zero: a moment still has to be visible. */
  width: number
  paused: boolean
  /** What the band is, for the hover title. */
  label: string
}

const MIN_BAND_WIDTH = 0.6

/**
 * The day as alternating worked and paused bands.
 *
 * Positions are relative to the day's own start and end, not to a fixed
 * 00:00-24:00 axis: a day is read against itself, and an eight-hour day drawn
 * across a third of the width wastes the space where the breaks are.
 *
 * A day whose pauses were not stored has none to draw - the caller says so in
 * words instead, rather than showing an unbroken bar that would read as
 * uninterrupted work.
 */
export function bands(day: Day): Band[] {
  const start = new Date(day.started_at).getTime()
  const end = dayEnd(day)
  if (end === null || end <= start) return []

  const span = end - start
  const bands: Band[] = []
  let cursor = start

  // Sorted defensively: the endpoint orders pauses, but a band laid out from
  // an unsorted list would silently overlap its neighbour.
  const pauses = [...day.pauses].sort((a, b) => new Date(a.started_at).getTime() - new Date(b.started_at).getTime())

  for (const pause of pauses) {
    const pauseStart = new Date(pause.started_at).getTime()
    const pauseEnd = pause.ended_at ? new Date(pause.ended_at).getTime() : pauseStart + (pause.duration_seconds ?? 0) * 1000
    // A pause recorded outside its day's bounds is the agent's business, not
    // something to draw off the end of the bar.
    const from = Math.max(start, Math.min(pauseStart, end))
    const to = Math.max(from, Math.min(pauseEnd, end))

    if (from > cursor) {
      bands.push(band(cursor - start, from - cursor, span, false, 'worked'))
    }
    bands.push(band(from - start, to - from, span, true, pause.manual ? 'break' : 'idle'))
    cursor = Math.max(cursor, to)
  }

  if (cursor < end) {
    bands.push(band(cursor - start, end - cursor, span, false, 'worked'))
  }

  return bands
}

/**
 * The moment a day's bar runs to.
 *
 * A finished day ends where it ended. A day still running has no end, but it
 * does have a shape worth drawing: the breaks already taken. It is drawn up to
 * the last thing the server knows about - never to "now", which would grow the
 * bar while nobody is working and imply the server had heard from the agent
 * since. `null` when there is nothing after the start to draw towards.
 */
function dayEnd(day: Day): number | null {
  if (day.ended_at) return new Date(day.ended_at).getTime()

  const known = day.pauses.flatMap((pause) => {
    const started = new Date(pause.started_at).getTime()
    const ended = pause.ended_at ? new Date(pause.ended_at).getTime() : started + (pause.duration_seconds ?? 0) * 1000
    return [started, ended]
  })
  if (known.length === 0) return null

  return Math.max(...known)
}

function band(offset: number, width: number, span: number, paused: boolean, label: string): Band {
  return {
    left: (offset / span) * 100,
    // A ten-second pause is real and would otherwise render as nothing at all.
    width: Math.max((width / span) * 100, MIN_BAND_WIDTH),
    paused,
    label,
  }
}
