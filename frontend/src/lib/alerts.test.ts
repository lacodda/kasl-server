import { describe, expect, it } from 'vitest'
import type { Alert, AlertRule } from '@/lib/api'
import { alertPhrase, alertTone, hours, isOpen, span } from '@/lib/alerts'

function alert(rule: AlertRule, overrides: Partial<Alert> = {}): Alert {
  return {
    id: 'a1',
    user_id: 'u1',
    display_name: 'Ann',
    department: null,
    rule,
    state: 'open',
    fired_at: '2026-09-18T09:00:00Z',
    resolved_at: null,
    acknowledged_at: null,
    acknowledged_by: null,
    observed_seconds: 0,
    against_seconds: null,
    subject_date: null,
    ...overrides,
  }
}

describe('hours', () => {
  it('reads as a bare number, and matches what the signals print', () => {
    // The two sit on one screen. "12.5 h" beside "12 h 30 m" reads as two
    // measurements of two different things.
    expect(hours(8 * 3600)).toBe('8')
    expect(hours(12.5 * 3600)).toBe('12.5')
  })

  it('never turns a missing figure into a confident zero', () => {
    expect(hours(null)).toBe('—')
    expect(hours(undefined)).toBe('—')
    // And a real zero is still a zero.
    expect(hours(0)).toBe('0')
  })
})

describe('span', () => {
  const HOUR = 3600
  const DAY = 24 * HOUR

  it('stays in hours while hours are still readable', () => {
    expect(span(9 * HOUR)).toBe('9 h')
    expect(span(30 * HOUR)).toBe('30 h')
    expect(span(47 * HOUR)).toBe('47 h')
  })

  it('switches to larger units where hours stop being a number anybody reads', () => {
    // A real stand carried a day open since the previous August. In hours
    // that is `9460.7 h`, which is true and useless.
    expect(span(2 * DAY)).toBe('2 d')
    expect(span(8 * DAY)).toBe('8 d')
    expect(span(45 * DAY)).toBe('1 mo')
    expect(span(394 * DAY)).toBe('1 y')
  })

  it('leaves no span without a unit', () => {
    // Steps on different divisors are how a gap appears: months of 30 days
    // and years of 365 leave days 360-364 belonging to neither unless the
    // thresholds are ordered so every value falls through into one. Checked
    // by sweeping the whole range rather than by reading the branches, which
    // is what missed it elsewhere in the line.
    for (let h = 0; h <= 400 * 24; h += 1) {
      const rendered = span(h * HOUR)
      expect(rendered, `${h} h rendered as \`${rendered}\``).toMatch(/^\d+(\.\d)? (h|d|mo|y)$/)
    }
  })

  it('never turns a missing figure into a confident zero', () => {
    expect(span(null)).toBe('—')
    expect(span(undefined)).toBe('—')
  })
})

describe('alertTone', () => {
  it('reserves the louder tone for the one that is a problem with the installation', () => {
    // An agent that stopped reporting makes every other number about that
    // person untrustworthy too, which the other two rules are not.
    expect(alertTone('no_agent_data')).toBe('warn')
    expect(alertTone('overwork')).toBe('info')
    expect(alertTone('day_not_closed')).toBe('info')
  })

  it('never paints an alert as a finding against a person', () => {
    // No `bad`. A row in red would make the screen an accusation rather than
    // a place to start a conversation - the same rule the signals follow.
    const tones = (['no_agent_data', 'overwork', 'day_not_closed'] as AlertRule[]).map(alertTone)
    expect(tones).not.toContain('bad')
  })
})

describe('alertPhrase', () => {
  it('quotes the silence, not the threshold it passed', () => {
    const phrase = alertPhrase(alert('no_agent_data', { observed_seconds: 30 * 3600, against_seconds: 12 * 3600 }))
    expect(phrase.key).toBe('alerts.noAgentData')
    // Thirty, not twelve. A sentence that read the threshold back would be
    // grammatical, identical on every row, and wrong.
    expect(phrase.values.span).toBe('30 h')
  })

  it('puts the hours worked next to the norm they are measured against', () => {
    const phrase = alertPhrase(
      alert('overwork', { observed_seconds: 12.5 * 3600, against_seconds: 8 * 3600, subject_date: '2026-09-17' }),
    )
    expect(phrase.key).toBe('alerts.overwork')
    expect(phrase.values).toMatchObject({ worked: '12.5', norm: '8', date: '2026-09-17' })
  })

  it('carries a part-timer their own norm rather than the installation day', () => {
    // The whole reason the server sends both figures: eight hours is an
    // ordinary day against a norm of eight, and double one against a norm of
    // four. A screen showing only the first cannot tell those apart.
    const phrase = alertPhrase(alert('overwork', { observed_seconds: 8 * 3600, against_seconds: 4 * 3600 }))
    expect(phrase.values).toMatchObject({ worked: '8', norm: '4' })
  })

  it('says how long a day has been open, and which day', () => {
    const phrase = alertPhrase(alert('day_not_closed', { observed_seconds: 32 * 3600, subject_date: '2026-09-18' }))
    expect(phrase.key).toBe('alerts.dayNotClosed')
    expect(phrase.values).toMatchObject({ span: '32 h', date: '2026-09-18' })
  })

  it('has a phrase for every rule the server can send', () => {
    // A rule with no sentence would render as its own i18n key on the screen.
    for (const rule of ['no_agent_data', 'overwork', 'day_not_closed'] as AlertRule[]) {
      expect(alertPhrase(alert(rule)).key).toMatch(/^alerts\./)
    }
  })
})

describe('isOpen', () => {
  it('counts only what is still waiting for somebody', () => {
    expect(isOpen(alert('overwork'))).toBe(true)
    // An acknowledged alert may still be perfectly true - it is answered, not
    // gone - and it must not sit in the badge.
    expect(isOpen(alert('overwork', { state: 'acknowledged', acknowledged_at: '2026-09-18T10:00:00Z' }))).toBe(false)
    expect(isOpen(alert('overwork', { state: 'resolved', resolved_at: '2026-09-18T10:00:00Z' }))).toBe(false)
  })
})
