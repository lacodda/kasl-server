import { describe, expect, it } from 'vitest'
import type { TimesheetCell, TimesheetColumn, TimesheetRow } from '@/lib/api'
import { busiest, currentMonth, dayOfMonth, isMonth, scale, shiftMonth, squares, step } from '@/lib/heatmap'

function cell(date: string, overrides: Partial<TimesheetCell> = {}): TimesheetCell {
  return { date, state: 'unrecorded', kind: null, worked_seconds: null, norm_seconds: 28_800, filed: 'missing', report: null, ...overrides }
}

function column(date: string, hours = 8): TimesheetColumn {
  return { date, hours, calendar: null, note: null, weekend: false }
}

function row(days: TimesheetCell[]): TimesheetRow {
  return { days } as TimesheetRow
}

const dates = ['2026-03-02', '2026-03-03', '2026-03-04']
const columns = dates.map((date) => column(date))

describe('squares', () => {
  it('tells a day with no data from a day of no hours', () => {
    // The distinction the whole view rests on. A date with nothing recorded is
    // not a worked day of zero, and a grid that painted them alike would
    // accuse someone of a month off for never installing the agent.
    const grid = squares(
      row([cell(dates[0]!), cell(dates[1]!, { state: 'worked', kind: 'work', worked_seconds: 0, filed: 'on_time' }), cell(dates[2]!, { state: 'ahead', filed: null })]),
      columns,
      28_800,
    )

    // Asserted first, because every `grid[n]?.` below is vacuously true on an
    // empty array: a square that never got drawn would pass all of them.
    expect(grid).toHaveLength(dates.length)
    expect(grid[0]?.kind).toBe('none')
    expect(grid[0]?.seconds).toBeNull()
    expect(grid[1]?.kind).toBe('worked')
    expect(grid[1]?.seconds).toBe(0)
    expect(grid[2]?.kind).toBe('none')
  })

  it('gives an open day no figure', () => {
    const grid = squares(row([cell(dates[0]!, { state: 'open', kind: 'work', filed: null })]), columns, 28_800)

    expect(grid[0]?.kind).toBe('open')
    expect(grid[0]?.seconds).toBeNull()
    expect(grid[0]?.intensity).toBeNull()
  })

  it('treats a finished day without a total as open', () => {
    // Belt and braces on the one field a screen must never guess at.
    const grid = squares(row([cell(dates[0]!, { state: 'worked', kind: 'work', worked_seconds: null })]), columns, 28_800)
    expect(grid[0]?.kind).toBe('open')
  })

  it('draws a day away as away, never as a short day worked', () => {
    // Before 0.28 a sick day was a square of zero hours, shaded as the
    // shortest working day of the month.
    const grid = squares(row([cell(dates[0]!, { state: 'away', kind: 'sick', worked_seconds: 0, norm_seconds: 0, filed: null })]), columns, 28_800)
    expect(grid[0]?.kind).toBe('away')
    expect(grid[0]?.away).toBe('sick')
    expect(grid[0]?.intensity).toBeNull()
  })

  it('scales every row against the same ceiling', () => {
    // Two people, one grid. A per-row scale would give each their own darkest
    // square and make a light week look like a heavy one.
    const worked = (seconds: number) => row([cell(dates[0]!, { state: 'worked', kind: 'work', worked_seconds: seconds })])
    expect(squares(worked(28_800), columns, 28_800)[0]?.intensity).toBe(1)
    expect(squares(worked(7_200), columns, 28_800)[0]?.intensity).toBeCloseTo(0.25)
  })

  it('marks rest days from the calendar, not from the weekday', () => {
    // A holiday on a Wednesday is a rest day, and a Saturday moved into the
    // working week is not: the column's hours say which, where the date
    // alone would get both wrong.
    const grid = squares(row([cell(dates[0]!, { state: 'free' }), cell(dates[1]!)]), [column(dates[0]!, 0), column(dates[1]!, 8)], null)
    expect(grid[0]?.rest).toBe(true)
    expect(grid[1]?.rest).toBe(false)
  })
})

describe('busiest', () => {
  it('is the longest finished day of work anywhere in the month', () => {
    const rows = [
      row([cell(dates[0]!, { state: 'worked', kind: 'work', worked_seconds: 30_000 })]),
      row([cell(dates[0]!, { state: 'worked', kind: 'work', worked_seconds: 36_000 }), cell(dates[1]!, { state: 'open', kind: 'work' })]),
    ]
    expect(busiest(rows)).toBe(36_000)
  })

  it('never takes a day away, and is nothing when nobody finished a day', () => {
    const away = row([cell(dates[0]!, { state: 'away', kind: 'vacation', worked_seconds: 50_000 })])
    expect(busiest([away])).toBeNull()
    expect(busiest([])).toBeNull()
  })
})

describe('scale', () => {
  it('keeps a very short day visible', () => {
    // Twenty minutes against an eight-hour ceiling is 0.04 - a square nobody
    // would see, which reads as "no data" and is the one lie this view must
    // not tell.
    expect(scale(1_200, 28_800)).toBe(0.15)
  })

  it('never overflows the top of the scale', () => {
    expect(scale(36_000, 28_800)).toBe(1)
  })

  it('answers a full square when there is no ceiling to compare against', () => {
    expect(scale(28_800, null)).toBe(1)
    expect(scale(28_800, 0)).toBe(1)
  })
})

describe('step', () => {
  it('snaps an intensity onto one of five shades', () => {
    expect(step(0.05)).toBe(1)
    expect(step(0.2)).toBe(1)
    expect(step(0.21)).toBe(2)
    expect(step(0.6)).toBe(3)
    expect(step(1)).toBe(5)
  })
})

describe('shiftMonth', () => {
  it('walks backwards and forwards a month at a time', () => {
    expect(shiftMonth('2026-03', -1)).toBe('2026-02')
    expect(shiftMonth('2026-03', 1)).toBe('2026-04')
  })

  it('rolls over a year boundary in both directions', () => {
    // The arithmetic that forgets this shows an empty grid every January.
    expect(shiftMonth('2026-12', 1)).toBe('2027-01')
    expect(shiftMonth('2026-01', -1)).toBe('2025-12')
  })

  it('keeps the two-digit shape', () => {
    expect(shiftMonth('2026-10', -1)).toBe('2026-09')
  })
})

describe('currentMonth', () => {
  it('reads the month from the local calendar', () => {
    expect(currentMonth(new Date(2026, 8, 1))).toBe('2026-09')
    expect(currentMonth(new Date(2026, 0, 31))).toBe('2026-01')
  })
})

describe('isMonth', () => {
  it('takes a month and refuses anything else', () => {
    expect(isMonth('2026-09')).toBe(true)
    for (const bad of [null, '', '2026-9', '2026-13', '2026-00', '2026-09-01', 'September']) {
      expect(isMonth(bad)).toBe(false)
    }
  })
})

describe('dayOfMonth', () => {
  it('reads the column label off the date', () => {
    expect(dayOfMonth('2026-03-01')).toBe(1)
    expect(dayOfMonth('2026-03-31')).toBe(31)
  })
})
