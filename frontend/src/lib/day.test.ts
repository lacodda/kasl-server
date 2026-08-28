import { describe, expect, it } from 'vitest'
import type { Day, Pause } from '@/lib/api'
import { bands, duration, isoDate, shiftWeeks, since, startOfWeek, weekDates, weekdayName } from '@/lib/day'

function pause(from: string, to: string | null, seconds: number, manual = false): Pause {
  return { id: from, started_at: from, ended_at: to, duration_seconds: seconds, manual, reason: null }
}

function day(overrides: Partial<Day> = {}): Day {
  return {
    date: '2026-08-24',
    started_at: '2026-08-24T12:00:00Z',
    ended_at: '2026-08-24T21:00:00Z',
    worked_seconds: 30000,
    paused_count: 0,
    paused_seconds: 0,
    pauses: [],
    tasks: [],
    ...overrides,
  }
}

describe('duration', () => {
  it('reads as hours and minutes', () => {
    expect(duration(0)).toBe('0h 0m')
    expect(duration(30000)).toBe('8h 20m')
  })

  it('does not round into a sixtieth minute', () => {
    // 7h59m30s rounds the minutes to 60, which must carry rather than print.
    expect(duration(7 * 3600 + 59 * 60 + 30)).toBe('8h 0m')
  })

  it('shows a missing figure as a gap, not a zero', () => {
    // An open day has no total. `0h 0m` would claim no work was done.
    expect(duration(null)).toBe('—')
    expect(duration(undefined)).toBe('—')
  })
})

describe('week boundaries', () => {
  it('starts the week on Monday', () => {
    // A Wednesday and the Sunday after it belong to the same week.
    expect(isoDate(startOfWeek(new Date(2026, 7, 26)))).toBe('2026-08-24')
    expect(isoDate(startOfWeek(new Date(2026, 7, 30)))).toBe('2026-08-24')
    // A Monday is already the start of its own week.
    expect(isoDate(startOfWeek(new Date(2026, 7, 24)))).toBe('2026-08-24')
  })

  it('lists seven days and crosses a month boundary', () => {
    const dates = weekDates(startOfWeek(new Date(2026, 7, 31)))
    expect(dates).toEqual(['2026-08-31', '2026-09-01', '2026-09-02', '2026-09-03', '2026-09-04', '2026-09-05', '2026-09-06'])
  })

  it('steps by whole weeks in both directions', () => {
    const monday = startOfWeek(new Date(2026, 7, 26))
    expect(isoDate(shiftWeeks(monday, -1))).toBe('2026-08-17')
    expect(isoDate(shiftWeeks(monday, 1))).toBe('2026-08-31')
  })

  it('keeps a date on its own calendar day', () => {
    // The regression this guards: `new Date('2026-08-24')` is UTC midnight,
    // which west of Greenwich is the 23rd. Dates here are local by
    // construction, so a day never files itself under yesterday (ADR 0003).
    expect(isoDate(new Date(2026, 7, 24))).toBe('2026-08-24')
  })
})

describe('weekdayName', () => {
  it('follows the interface language, not the machine', () => {
    // The regression: `toLocaleDateString([])` follows the operating system,
    // which printed Russian weekdays in an otherwise English page. Only a look
    // at the screen caught it - every assertion about the layout still passed.
    expect(weekdayName('2026-08-24')).toBe('Mon')
    expect(weekdayName('2026-08-30')).toBe('Sun')
  })

  it('keeps a date on its own weekday in any zone', () => {
    // Midday rather than midnight: parsed at 00:00 a date can slide onto the
    // previous day west of Greenwich, and the row would be labelled Sunday.
    expect(weekdayName('2026-08-27')).toBe('Thu')
  })
})

describe('since', () => {
  const now = new Date('2026-08-28T12:00:00Z').getTime()
  const ago = (minutes: number) => new Date(now - minutes * 60000).toISOString()

  it('reports minutes, then hours, then days', () => {
    expect(since(ago(12), now)).toEqual(['minutes', 12])
    expect(since(ago(90), now)).toEqual(['hours', 2])
    expect(since(ago(60 * 72), now)).toEqual(['days', 3])
  })

  it('switches units at its boundaries', () => {
    // 59 minutes is still minutes; an hour is not.
    expect(since(ago(59), now)[0]).toBe('minutes')
    expect(since(ago(60), now)[0]).toBe('hours')
    // Hours run to two days, so yesterday afternoon stays legible as hours
    // rather than collapsing into a bare "1 d".
    expect(since(ago(60 * 47), now)[0]).toBe('hours')
    expect(since(ago(60 * 48), now)[0]).toBe('days')
  })

  it('never reports a future moment', () => {
    // A client clock a little ahead of the server's must read as "just now",
    // not as "-3 min", which would look like a bug in the server.
    expect(since(new Date(now + 3 * 60000).toISOString(), now)).toEqual(['minutes', 0])
  })
})

describe('bands', () => {
  it('splits a day around its pauses', () => {
    // 12:00-21:00 with a half-hour pause at 15:00: worked, paused, worked.
    const drawn = bands(day({ pauses: [pause('2026-08-24T15:00:00Z', '2026-08-24T15:30:00Z', 1800)] }))
    expect(drawn.map((band) => band.paused)).toEqual([false, true, false])
    const [first, paused] = drawn
    expect(first?.left).toBeCloseTo(0)
    // Three hours into a nine-hour day.
    expect(paused?.left).toBeCloseTo((3 / 9) * 100)
    expect(paused?.width).toBeCloseTo((0.5 / 9) * 100)
  })

  it('covers the day exactly once', () => {
    const drawn = bands(
      day({
        pauses: [pause('2026-08-24T15:00:00Z', '2026-08-24T15:30:00Z', 1800), pause('2026-08-24T18:00:00Z', '2026-08-24T18:10:00Z', 600)],
      }),
    )
    // Bands must tile the bar: a gap draws work that did not happen as a pause,
    // and an overlap hides one behind another.
    expect(drawn.length).toBe(5)
    drawn.slice(1).forEach((band, index) => {
      const previous = drawn[index]!
      expect(band.left).toBeCloseTo(previous.left + previous.width)
    })
    const last = drawn.at(-1)!
    expect(last.left + last.width).toBeCloseTo(100)
  })

  it('keeps a very short pause visible', () => {
    // Ten seconds in nine hours is 0.03% - invisible at any width, and the one
    // thing on the bar the employee might be looking for.
    const drawn = bands(day({ pauses: [pause('2026-08-24T15:00:00Z', '2026-08-24T15:00:10Z', 10)] }))
    const paused = drawn.find((band) => band.paused)
    expect(paused?.width).toBeGreaterThan(0.5)
  })

  it('orders pauses it was given out of order', () => {
    const drawn = bands(
      day({
        pauses: [pause('2026-08-24T18:00:00Z', '2026-08-24T18:10:00Z', 600), pause('2026-08-24T15:00:00Z', '2026-08-24T15:30:00Z', 1800)],
      }),
    )
    const positions = drawn.map((band) => band.left)
    expect([...positions].sort((a, b) => a - b)).toEqual(positions)
  })

  it('draws nothing for an open day with nothing in it yet', () => {
    // No end and no pauses: there is no span to lay bands out on.
    expect(bands(day({ ended_at: null, worked_seconds: null }))).toEqual([])
  })

  it('draws an open day up to what is known about it', () => {
    // The regression this guards was found by looking at the screen: a running
    // day with a lunch break drew an empty bar next to "1 · 0h 45m", which
    // contradicted itself. The bar runs to the last thing the server knows -
    // never to "now", which would grow while nobody is working.
    const drawn = bands(
      day({ ended_at: null, worked_seconds: null, pauses: [pause('2026-08-24T15:15:00Z', '2026-08-24T16:00:00Z', 2700, true)] }),
    )
    expect(drawn.map((band) => band.paused)).toEqual([false, true])
    const last = drawn.at(-1)!
    expect(last.left + last.width).toBeCloseTo(100)
  })

  it('clamps a pause recorded outside the day', () => {
    // The agent's business, not something to draw off the end of the bar.
    const drawn = bands(day({ pauses: [pause('2026-08-24T11:00:00Z', '2026-08-24T22:00:00Z', 39600)] }))
    for (const band of drawn) {
      expect(band.left).toBeGreaterThanOrEqual(0)
      expect(band.left + band.width).toBeLessThanOrEqual(100.01)
    }
  })
})
