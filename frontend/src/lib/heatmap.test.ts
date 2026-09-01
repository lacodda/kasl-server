import { describe, expect, it } from 'vitest'
import type { HeatmapRow } from '@/lib/api'
import { currentMonth, dayOfMonth, isWeekend, monthDates, scale, shiftMonth, squares, step } from '@/lib/heatmap'

function row(days: HeatmapRow['days'], overrides: Partial<HeatmapRow> = {}): HeatmapRow {
  return {
    user_id: 'u1',
    display_name: 'Ann',
    department: null,
    days,
    busiest_seconds: null,
    worked_seconds: 0,
    ...overrides,
  }
}

describe('monthDates', () => {
  it('covers a month end to end', () => {
    const dates = monthDates('2026-03-01', '2026-03-31')
    expect(dates).toHaveLength(31)
    expect(dates[0]).toBe('2026-03-01')
    expect(dates[30]).toBe('2026-03-31')
  })

  it('gets February right in both kinds of year', () => {
    expect(monthDates('2026-02-01', '2026-02-28')).toHaveLength(28)
    expect(monthDates('2024-02-01', '2024-02-29')).toHaveLength(29)
  })

  it('does not lose a day to a time zone', () => {
    // The whole reason dates are stepped through UTC: read in a western zone,
    // `2026-03-01` as a local moment is the previous February.
    expect(monthDates('2026-03-01', '2026-03-03')).toEqual(['2026-03-01', '2026-03-02', '2026-03-03'])
  })
})

describe('isWeekend', () => {
  it('knows Saturday and Sunday from the date alone', () => {
    // 2026-03-07 is a Saturday, the 8th a Sunday, the 9th a Monday.
    expect(isWeekend('2026-03-07')).toBe(true)
    expect(isWeekend('2026-03-08')).toBe(true)
    expect(isWeekend('2026-03-09')).toBe(false)
  })
})

describe('squares', () => {
  const dates = ['2026-03-02', '2026-03-03', '2026-03-04']

  it('tells a day with no data from a day of no hours', () => {
    // The distinction the whole screen rests on. A missing date is not a
    // worked day of zero, and a grid that painted them alike would accuse
    // someone of a month off for never installing the agent.
    const grid = squares(row([{ date: '2026-03-03', worked_seconds: 0, open: false }]), dates, 28_800)

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
    const grid = squares(row([{ date: '2026-03-02', worked_seconds: null, open: true }]), dates, 28_800)

    expect(grid[0]?.kind).toBe('open')
    expect(grid[0]?.seconds).toBeNull()
    expect(grid[0]?.intensity).toBeNull()
  })

  it('treats a missing total as open even when the flag says otherwise', () => {
    // Belt and braces on the one field a screen must never guess at: without
    // this, a `null` total would fall through to `worked` and render as a
    // square whose hours are unknown.
    const grid = squares(row([{ date: '2026-03-02', worked_seconds: null, open: false }]), dates, 28_800)
    expect(grid[0]?.kind).toBe('open')
  })

  it('scales every row against the same ceiling', () => {
    // Two people, one grid. A per-row scale would give each their own darkest
    // square and make a light week look like a heavy one.
    const busy = squares(row([{ date: '2026-03-02', worked_seconds: 28_800, open: false }]), dates, 28_800)
    const quiet = squares(row([{ date: '2026-03-02', worked_seconds: 7_200, open: false }]), dates, 28_800)

    expect(busy[0]?.intensity).toBe(1)
    expect(quiet[0]?.intensity).toBeCloseTo(0.25)
  })

  it('marks weekends from the date', () => {
    const grid = squares(row([]), ['2026-03-06', '2026-03-07'], null)
    expect(grid[0]?.weekend).toBe(false)
    expect(grid[1]?.weekend).toBe(true)
  })
})

describe('scale', () => {
  it('keeps a very short day visible', () => {
    // Twenty minutes against an eight-hour ceiling is 0.04 - a square nobody
    // would see, which reads as "no data" and is the one lie this screen
    // must not tell.
    expect(scale(1_200, 28_800)).toBe(0.15)
  })

  it('never overflows the top of the scale', () => {
    expect(scale(36_000, 28_800)).toBe(1)
  })

  it('answers a full square when there is no ceiling to compare against', () => {
    // One recorded day in the whole grid: it is the busiest by definition, and
    // a division by zero would paint it as nothing.
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

describe('dayOfMonth', () => {
  it('reads the column label off the date', () => {
    expect(dayOfMonth('2026-03-01')).toBe(1)
    expect(dayOfMonth('2026-03-31')).toBe(31)
  })
})
