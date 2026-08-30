import { describe, expect, it } from 'vitest'
import type { LiveStatus } from '@/lib/api'
import { statusTone } from '@/lib/live'

/**
 * The mapping from a status to how the row is drawn.
 *
 * Worth its own test because every mistake here is silent: a status that fell
 * through to a default colour, or one painted as a problem when it is not,
 * looks like a working dashboard to whoever is reading it.
 */
describe('how a live status is drawn', () => {
  it('separates being at work from merely being reachable', () => {
    // Only the two states inside a working day count as someone at work.
    // `idle` is an agent that is running and reporting - a healthy
    // installation, not a person, and a count that included it would tell a
    // manager their whole team is working at midnight.
    expect(statusTone('working').live).toBe(true)
    expect(statusTone('paused').live).toBe(true)
    for (const status of ['idle', 'offline', 'unknown'] as LiveStatus[]) {
      expect(statusTone(status).live, status).toBe(false)
    }
  })

  it('warns about a break and not about a quiet agent', () => {
    // The colours carry a judgement, so they are asserted rather than left to
    // a screenshot. Nobody should be alerted because a colleague finished
    // their day or has not upgraded kasl yet.
    expect(statusTone('working').tone).toBe('good')
    expect(statusTone('paused').tone).toBe('warn')
    expect(statusTone('idle').tone).toBe('dim')
    expect(statusTone('offline').tone).toBe('dim')
    expect(statusTone('unknown').tone).toBe('faint')
  })

  it('has an answer for every status the server can send', () => {
    // The guard against a status added on the server and forgotten here: a
    // missing case would return undefined and paint the row with no class at
    // all, which is invisible until someone notices a blank column.
    const all: LiveStatus[] = ['working', 'paused', 'idle', 'offline', 'unknown']
    for (const status of all) {
      const tone = statusTone(status)
      expect(tone, status).toBeDefined()
      expect(['good', 'warn', 'dim', 'faint'], status).toContain(tone.tone)
    }
  })
})
