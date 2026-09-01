import { describe, expect, it } from 'vitest'
import type { Signal, SignalKind, TrendWeek } from '@/lib/api'
import { bars, hours, signalPhrase, signalTone, weekLabel } from '@/lib/signals'

function signal(kind: SignalKind, overrides: Partial<Signal> = {}): Signal {
  return {
    user_id: 'u1',
    display_name: 'Ann',
    department: null,
    kind,
    weeks: null,
    from_seconds: null,
    to_seconds: null,
    median_seconds: null,
    days_quiet: null,
    ...overrides,
  }
}

function week(worked_seconds: number, days_recorded = 5, week_start = '2026-08-24'): TrendWeek {
  return { week_start, worked_seconds, days_recorded }
}

describe('hours', () => {
  it('reads as a bare number, one decimal where it needs one', () => {
    expect(hours(8 * 3600)).toBe('8')
    expect(hours(8.5 * 3600)).toBe('8.5')
    expect(hours(0)).toBe('0')
  })

  it('never renders a missing figure as a confident zero', () => {
    // The distinction the whole product keeps: absent is not nought.
    expect(hours(null)).toBe('—')
    expect(hours(undefined)).toBe('—')
  })
})

describe('signalPhrase', () => {
  it('quotes the figures a decline was measured from', () => {
    const phrase = signalPhrase(signal('declining', { weeks: 3, from_seconds: 40 * 3600, to_seconds: 25 * 3600 }))
    expect(phrase.key).toBe('signals.declining')
    expect(phrase.values).toEqual({ weeks: 3, from: '40', to: '25' })
  })

  it('says how long the silence has lasted', () => {
    const phrase = signalPhrase(signal('no_data', { days_quiet: 11 }))
    expect(phrase.key).toBe('signals.noData')
    expect(phrase.values).toEqual({ days: 11 })
  })

  it('tells an unusually long week from an unusually short one', () => {
    // Both directions get their own sentence. One phrase for both would have
    // to be vague enough to cover them, and vagueness here reads as suspicion.
    const low = signalPhrase(signal('unusual_week', { to_seconds: 20 * 3600, median_seconds: 40 * 3600 }))
    expect(low.key).toBe('signals.unusualLow')
    expect(low.values).toEqual({ week: '20', median: '40' })

    const high = signalPhrase(signal('unusual_week', { to_seconds: 60 * 3600, median_seconds: 40 * 3600 }))
    expect(high.key).toBe('signals.unusualHigh')
  })

  it('falls back to the low phrasing when a figure is missing', () => {
    // Never the "worked more than usual" sentence on a guess: it is the one
    // that reads as praise, and praise for a figure nobody sent is worse than
    // a neutral line.
    const phrase = signalPhrase(signal('unusual_week'))
    expect(phrase.key).toBe('signals.unusualLow')
    expect(phrase.values).toEqual({ week: '—', median: '—' })
  })
})

describe('signalTone', () => {
  it('never paints a person as a problem', () => {
    // No signal is a finding against anybody. `bad` would make the screen an
    // accusation rather than a place a conversation starts.
    for (const kind of ['declining', 'no_data', 'unusual_week'] as const) {
      expect(['warn', 'info']).toContain(signalTone(kind))
    }
  })

  it('warns only about the installation, not about the person', () => {
    // Silence is the one that says something is broken: an agent that stopped
    // makes every other number about that person untrustworthy.
    expect(signalTone('no_data')).toBe('warn')
    expect(signalTone('declining')).toBe('info')
    expect(signalTone('unusual_week')).toBe('info')
  })
})

describe('bars', () => {
  it('scales every bar against the tallest week', () => {
    const drawn = bars([week(40 * 3600), week(20 * 3600), week(10 * 3600)])
    expect(drawn).toHaveLength(3)
    expect(drawn[0]?.height).toBeCloseTo(1)
    expect(drawn[1]?.height).toBeCloseTo(0.5)
    expect(drawn[2]?.height).toBeCloseTo(0.25)
  })

  it('keeps an empty week in its place and marks it', () => {
    // Dropping it would close the gap up and turn an absence into continuity -
    // the one thing the chart exists to show.
    const drawn = bars([week(40 * 3600), week(0, 0), week(40 * 3600)])
    expect(drawn).toHaveLength(3)
    expect(drawn[1]?.empty).toBe(true)
    expect(drawn[1]?.height).toBe(0)
    expect(drawn[0]?.empty).toBe(false)
  })

  it('does not divide by zero when nothing was worked at all', () => {
    const drawn = bars([week(0, 0), week(0, 0)])
    expect(drawn.every((bar) => Number.isFinite(bar.height))).toBe(true)
    expect(drawn.every((bar) => bar.height === 0)).toBe(true)
  })

  it('separates a week of no hours from a week of no data', () => {
    // A week with days recorded but nothing worked - every day still open -
    // is not the same as a week nobody filed, and only `empty` says which.
    const drawn = bars([week(0, 5), week(0, 0)])
    expect(drawn[0]?.empty).toBe(false)
    expect(drawn[1]?.empty).toBe(true)
  })
})

describe('weekLabel', () => {
  it('names the week by the day it starts on', () => {
    expect(weekLabel('2026-08-24', 'en')).toContain('24')
  })

  it('does not slip onto the previous day in a western zone', () => {
    // The same rule the rest of the app follows: a date is a label, and
    // reading it as UTC midnight files it under yesterday west of Greenwich.
    expect(weekLabel('2026-03-01', 'en')).toContain('1')
  })
})
