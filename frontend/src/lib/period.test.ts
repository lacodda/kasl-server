import { describe, expect, it } from 'vitest'
import { elapsed, percent, periodDates, periodFromParams, periodOf, periodParams, shareChange, shareOfNorm, shiftPeriod } from '@/lib/period'

describe('periodOf', () => {
  it('a day is itself', () => {
    expect(periodOf('day', '2026-10-07')).toEqual({ unit: 'day', from: '2026-10-07', to: '2026-10-07' })
  })

  it('a week runs Monday to Sunday, and a Sunday belongs to the week it ends', () => {
    expect(periodOf('week', '2026-10-07')).toEqual({ unit: 'week', from: '2026-10-05', to: '2026-10-11' })
    expect(periodOf('week', '2026-10-11')).toEqual({ unit: 'week', from: '2026-10-05', to: '2026-10-11' })
    expect(periodOf('week', '2026-10-05')).toEqual({ unit: 'week', from: '2026-10-05', to: '2026-10-11' })
  })

  it('a week can straddle two months and two years', () => {
    expect(periodOf('week', '2026-12-31')).toEqual({ unit: 'week', from: '2026-12-28', to: '2027-01-03' })
  })

  it('a month runs from its first to its last day, February included', () => {
    expect(periodOf('month', '2026-10-15')).toEqual({ unit: 'month', from: '2026-10-01', to: '2026-10-31' })
    expect(periodOf('month', '2028-02-10')).toEqual({ unit: 'month', from: '2028-02-01', to: '2028-02-29' })
    expect(periodOf('month', '2026-02-28')).toEqual({ unit: 'month', from: '2026-02-01', to: '2026-02-28' })
  })
})

describe('shiftPeriod', () => {
  it('steps by the unit, both ways', () => {
    expect(shiftPeriod(periodOf('day', '2026-10-01'), -1).from).toBe('2026-09-30')
    expect(shiftPeriod(periodOf('week', '2026-10-07'), -1)).toEqual({ unit: 'week', from: '2026-09-28', to: '2026-10-04' })
    expect(shiftPeriod(periodOf('week', '2026-10-07'), 2).from).toBe('2026-10-19')
  })

  it('moves a month by months, not by thirty days', () => {
    // From March, thirty days back lands in February's neighbour; a month back
    // is February itself, all of it.
    expect(shiftPeriod(periodOf('month', '2026-03-31'), -1)).toEqual({ unit: 'month', from: '2026-02-01', to: '2026-02-28' })
    expect(shiftPeriod(periodOf('month', '2026-12-05'), 1)).toEqual({ unit: 'month', from: '2027-01-01', to: '2027-01-31' })
  })
})

describe('periodDates', () => {
  it('lists every date, both ends included', () => {
    expect(periodDates(periodOf('week', '2026-10-07'))).toEqual([
      '2026-10-05',
      '2026-10-06',
      '2026-10-07',
      '2026-10-08',
      '2026-10-09',
      '2026-10-10',
      '2026-10-11',
    ])
    expect(periodDates(periodOf('month', '2026-02-01'))).toHaveLength(28)
    expect(periodDates(periodOf('day', '2026-10-07'))).toEqual(['2026-10-07'])
  })
})

describe('periodFromParams', () => {
  const params = (query: string) => new URLSearchParams(query)

  it('reads the unit and normalizes the date to the period start', () => {
    expect(periodFromParams(params('period=month&from=2026-09-17'), '2026-10-10')).toEqual({
      unit: 'month',
      from: '2026-09-01',
      to: '2026-09-30',
    })
  })

  it('falls back to this period of the default unit', () => {
    expect(periodFromParams(params(''), '2026-10-10')).toEqual(periodOf('week', '2026-10-10'))
    expect(periodFromParams(params(''), '2026-10-10', 'month')).toEqual(periodOf('month', '2026-10-10'))
  })

  it('ignores what is not a unit or not a date', () => {
    expect(periodFromParams(params('period=year&from=yesterday'), '2026-10-10')).toEqual(periodOf('week', '2026-10-10'))
    // Shaped like a date, and not one.
    expect(periodFromParams(params('period=day&from=2026-02-30'), '2026-10-10')).toEqual(periodOf('day', '2026-10-10'))
  })

  it('opens the period around a linked day', () => {
    expect(periodFromParams(params('date=2026-09-17'), '2026-10-10')).toEqual(periodOf('week', '2026-09-17'))
  })

  it('writes back what it reads', () => {
    const period = periodOf('month', '2026-09-17')
    expect(periodFromParams(new URLSearchParams(periodParams(period)), '2026-10-10')).toEqual(period)
  })
})

describe('shares', () => {
  it('is a fraction of the norm, and nothing where nothing was owed', () => {
    expect(shareOfNorm(30 * 3600, 40 * 3600)).toBe(0.75)
    expect(shareOfNorm(5 * 3600, 0)).toBeNull()
  })

  it('compares in whole percentage points, or not at all', () => {
    expect(shareChange(0.9, 0.75)).toBe(15)
    expect(shareChange(0.75, 0.9)).toBe(-15)
    expect(shareChange(0.9, null)).toBeNull()
    expect(shareChange(null, 0.9)).toBeNull()
  })

  it('prints a whole percentage', () => {
    expect(percent(0.875)).toBe('88%')
    expect(percent(1.2)).toBe('120%')
  })
})

describe('elapsed', () => {
  const october = periodOf('month', '2026-10-01')

  it('is the whole of a period that is over', () => {
    expect(elapsed(periodOf('month', '2026-09-01'), '2026-10-10')).toEqual({ from: '2026-09-01', to: '2026-09-30' })
  })

  it('stops at today in a period still running', () => {
    expect(elapsed(october, '2026-10-10')).toEqual({ from: '2026-10-01', to: '2026-10-10' })
    // Its first day and its last are both still the period.
    expect(elapsed(october, '2026-10-01')).toEqual({ from: '2026-10-01', to: '2026-10-01' })
    expect(elapsed(october, '2026-10-31')).toEqual({ from: '2026-10-01', to: '2026-10-31' })
  })

  it('is nothing for a period that has not begun', () => {
    expect(elapsed(october, '2026-09-30')).toBeNull()
  })
})
