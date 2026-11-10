export {}
// Code first, so the description below survives `shadcn add`: the CLI writes
// a file from its first token on and drops every comment above it.

/*
 * The arithmetic a calendar runs on, with no React in it.
 *
 * Split out of the Calendar because the size gate asked the right question:
 * the file was two and a half times over its ceiling, and the reason was that
 * it held two things - the sums, and the grid that draws them. These are the
 * sums, and they are what DatePicker, DateRangePicker and any product doing
 * its own date work import.
 *
 * Everything here takes and returns `YYYY-MM-DD`, and never a `Date`. A
 * birthday has no timezone; a release date has no hour. Put one in a `Date`
 * and it becomes a moment - and moments cross midnight when they are
 * serialised, which is how a date reaches a server a day early. The string is
 * what a database column holds and what JSON carries.
 *
 * `Date` appears inside, in two places only: to ask `Intl` for a name, and to
 * add days. Both are wrapped here, so no caller ever holds one.
 *
 * No date library, deliberately. `react-day-picker` is good and would bring
 * `date-fns` and `@date-fns/tz` behind it - the first heavy dependency in a
 * set that is otherwise Base UI or nothing. `Intl` already knows the part a
 * library would be consulted for: which day the week starts on here, and what
 * the months are called. The rest is the arithmetic below, and it is only
 * hard when a date is stored as a moment.
 */

/** A calendar date: `2026-09-02`. Not a moment - no time, no zone. */
export type IsoDate = string

/** Whether a string is a calendar date this component can work with, and one
 * that actually exists. `2026-02-31` parses arithmetically and is not a day. */
export function isIsoDate(value: string): value is IsoDate {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false
  const [year, month, day] = value.split('-').map(Number) as [number, number, number]
  if (month < 1 || month > 12 || day < 1) return false
  return day <= daysInMonth(year, month)
}

/** How many days that month has. The leap rule in full, because the
 * hundred-year exception is the part that gets left out. */
export function daysInMonth(year: number, month: number): number {
  if (month === 2) {
    const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0
    return leap ? 29 : 28
  }
  return [4, 6, 9, 11].includes(month) ? 30 : 31
}

/** Today, as a calendar date in the reader's own timezone.
 *
 * Deliberately not `new Date().toISOString().slice(0, 10)`, which is the
 * common spelling and is wrong: that converts to UTC first, so anyone east of
 * Greenwich late in the evening gets tomorrow. */
export function today(): IsoDate {
  const now = new Date()
  return format(now.getFullYear(), now.getMonth() + 1, now.getDate())
}

function format(year: number, month: number, day: number): IsoDate {
  return `${String(year).padStart(4, '0')}-${String(month).padStart(2, '0')}-${String(day).padStart(2, '0')}`
}

interface Parts {
  year: number
  month: number
  day: number
}

export function parts(date: IsoDate): Parts {
  const [year, month, day] = date.split('-').map(Number) as [number, number, number]
  return { year, month, day }
}

/** The same date shifted by whole days. Goes through a `Date` at noon rather
 * than midnight: a shift over a daylight-saving boundary at midnight can land
 * on the same calendar day it started from. */
export function addDays(date: IsoDate, days: number): IsoDate {
  const { year, month, day } = parts(date)
  const moved = new Date(year, month - 1, day, 12)
  moved.setDate(moved.getDate() + days)
  return format(moved.getFullYear(), moved.getMonth() + 1, moved.getDate())
}

/** The same day-of-month in another month, clamped when it does not exist
 * there: a step back from 31 March lands on 28 February, not on 3 March. */
export function addMonths(date: IsoDate, months: number): IsoDate {
  const { year, month, day } = parts(date)
  const zero = year * 12 + (month - 1) + months
  const nextYear = Math.floor(zero / 12)
  const nextMonth = (zero % 12) + 1
  return format(nextYear, nextMonth, Math.min(day, daysInMonth(nextYear, nextMonth)))
}

/** Which weekday a date falls on, as `Intl` numbers them: 1 is Monday, 7 is
 * Sunday. `Date` numbers Sunday 0, which does not sort and does not match
 * what `getWeekInfo` returns. */
export function weekday(date: IsoDate): number {
  const { year, month, day } = parts(date)
  const js = new Date(year, month - 1, day, 12).getDay()
  return js === 0 ? 7 : js
}

/** Which day the week starts on here: 1 Monday, 7 Sunday.
 *
 * `getWeekInfo` is the current spelling and `weekInfo` the older one; some
 * engines have neither, and Monday is the majority answer worldwide. */
export function firstDayOfWeek(locale: string): number {
  try {
    const info = new Intl.Locale(locale) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number }
      weekInfo?: { firstDay: number }
    }
    return info.getWeekInfo?.().firstDay ?? info.weekInfo?.firstDay ?? 1
  } catch {
    return 1
  }
}

/** The grid of a month: whole weeks, starting on the locale's first day, with
 * the days either side included so every row has seven.
 *
 * Returned as dates rather than as numbers, so a cell never has to be told
 * which month it belongs to - it knows, and a click on a trailing day works
 * without a special case. */
export function monthGrid(month: IsoDate, locale: string): IsoDate[][] {
  const { year, month: monthNumber } = parts(month)
  const first = format(year, monthNumber, 1)
  const start = firstDayOfWeek(locale)

  // How far back the grid starts: the distance from the first of the month
  // back to the most recent week start.
  const lead = (weekday(first) - start + 7) % 7
  const origin = addDays(first, -lead)

  const weeks: IsoDate[][] = []
  let cursor = origin
  // Six rows always, so the calendar does not change height between months -
  // a popup that resizes as you page through it is one that moves under the
  // pointer.
  for (let week = 0; week < 6; week += 1) {
    const row: IsoDate[] = []
    for (let day = 0; day < 7; day += 1) {
      row.push(cursor)
      cursor = addDays(cursor, 1)
    }
    weeks.push(row)
  }
  return weeks
}

/** The weeks a month actually touches: the rows of `monthGrid` holding at
 * least one of its days - four to six of them.
 *
 * The Calendar keeps six rows always, because a popup that changes height
 * moves under the pointer. A month filling a screen has the opposite problem:
 * a sixth row made entirely of the next month makes every month look six
 * weeks long, and takes a sixth of the height from the days that are in it. */
export function monthWeeks(month: IsoDate, locale: string): IsoDate[][] {
  const { year, month: monthNumber } = parts(month)
  return monthGrid(month, locale).filter((week) =>
    week.some((date) => {
      const day = parts(date)
      return day.year === year && day.month === monthNumber
    }),
  )
}

/** Where a calendar's cursor goes for a key, or `undefined` for a key that
 * does not move it. `weekStart` is the locale's first day, which is where
 * Home lands.
 *
 * One map for every grid of days in the set, so the Calendar and the month
 * view cannot come to disagree about what PageDown means. */
export function keyStep(key: string, from: IsoDate, weekStart: number): IsoDate | undefined {
  const intoWeek = (weekday(from) - weekStart + 7) % 7
  const steps: Record<string, () => IsoDate> = {
    ArrowRight: () => addDays(from, 1),
    ArrowLeft: () => addDays(from, -1),
    ArrowDown: () => addDays(from, 7),
    ArrowUp: () => addDays(from, -7),
    PageDown: () => addMonths(from, 1),
    PageUp: () => addMonths(from, -1),
    Home: () => addDays(from, -intoWeek),
    End: () => addDays(from, 6 - intoWeek),
  }
  return steps[key]?.()
}

/** The weekday initials, in the order this locale lays them out.
 *
 * The week below is midnight in UTC, so it is named in UTC. Named in the
 * reader's own zone, as it was until v0.34, every reader west of Greenwich saw
 * each midnight as the evening before: a Monday-first grid headed "Sun Mon
 * Tue", one column off from the days under it. The tests ran in UTC, where
 * the two agree, and passed. */
export function weekdayNames(locale: string, start: number): string[] {
  const names = new Intl.DateTimeFormat(locale, { weekday: 'short', timeZone: 'UTC' })
  // Any week works; this one begins on a Monday.
  const monday = Date.UTC(2024, 0, 1)
  return Array.from({ length: 7 }, (_, index) => {
    const offset = (start - 1 + index) % 7
    return names.format(new Date(monday + offset * 86_400_000))
  })
}
