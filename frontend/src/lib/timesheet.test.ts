import { describe, expect, it } from 'vitest'
import type { TimesheetRow } from '@/lib/api'
import { daysAway, hoursShort, measuredAgainst, standingOf, viewOf } from '@/lib/timesheet'

describe('viewOf', () => {
  it('reads the three views and falls back to the timesheet', () => {
    expect(viewOf('reports')).toBe('reports')
    expect(viewOf('shape')).toBe('shape')
    expect(viewOf('timesheet')).toBe('timesheet')
    // A link from before the views, or a typo, lands on the record rather
    // than on a blank screen.
    expect(viewOf(null)).toBe('timesheet')
    expect(viewOf('heatmap')).toBe('timesheet')
  })
})

describe('hoursShort', () => {
  it('writes tenths, and no point when there is nothing after it', () => {
    expect(hoursShort(27_000)).toBe('7.5')
    expect(hoursShort(28_800)).toBe('8')
    expect(hoursShort(1_080)).toBe('0.3')
    expect(hoursShort(0)).toBe('0')
  })

  it('rounds to the nearest tenth rather than truncating', () => {
    // 7h 57m is 7.95 h: a cell reading 7.9 under a total that rounds up would
    // not add up across the row.
    expect(hoursShort(7 * 3600 + 57 * 60)).toBe('8')
    expect(hoursShort(7 * 3600 + 51 * 60)).toBe('7.9')
  })
})

describe('standingOf', () => {
  it('says what a manager made of a report', () => {
    expect(standingOf('approved')).toBe('approved')
    // Sent back, or moved since it was reported: either way the person has
    // something to do, and the cell says so the same way.
    expect(standingOf('returned')).toBe('attention')
    expect(standingOf('changed')).toBe('attention')
    expect(standingOf('submitted')).toBeNull()
    expect(standingOf(null)).toBeNull()
  })
})

describe('totals', () => {
  const row = {
    due_seconds: 3600,
    norm_seconds: 7200,
    counts: { vacation: 2, sick: 1, day_off: 1, unrecorded: 0, on_time: 0, late: 0, missing: 0, pending: 0, approved: 0 },
  } as TimesheetRow

  it('measures a row against what has come due, not the whole month', () => {
    expect(measuredAgainst(row)).toBe(3600)
  })

  it('counts every kind of day away', () => {
    expect(daysAway(row)).toBe(4)
  })
})
