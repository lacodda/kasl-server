import { describe, expect, it } from 'vitest'
import type { Alert, AlertRule } from '@/lib/api'
import { alertPhrase, alertTone, hours, isOpen } from '@/lib/alerts'

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
    expect(phrase.values.hours).toBe('30')
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
    expect(phrase.values).toMatchObject({ hours: '32', date: '2026-09-18' })
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
