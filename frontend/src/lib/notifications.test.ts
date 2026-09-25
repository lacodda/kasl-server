import { describe, expect, it } from 'vitest'
import type { Notification } from '@/lib/api'
import { noticeState, readThrough, screenFor, segments } from '@/lib/notifications'

const notice = (fields: Partial<Notification>): Notification => ({
  id: 1,
  kind: 'alert.raised',
  created_at: '2026-09-25T08:00:00Z',
  title: 'Your day of 2026-09-22 is still open here',
  body: 'This server has had it open for 17 h.',
  withdrawn_at: null,
  read: false,
  ...fields,
})

describe('noticeState', () => {
  it('draws what is over as over, new or not', () => {
    // The day closed: the notice stays a fact, and must not ask for attention
    // it no longer needs - the badge leaves it out for the same reason.
    expect(noticeState(notice({ id: 5, withdrawn_at: '2026-09-25T09:00:00Z' }), 0)).toBe('over')
    expect(noticeState(notice({ id: 5, withdrawn_at: '2026-09-25T09:00:00Z' }), 9)).toBe('over')
  })

  it('tells new from read by the cursor the screen was opened at', () => {
    // Not by the server's `read` flag: opening the screen marks everything
    // read a moment later, and the new ones must stay marked for this visit.
    expect(noticeState(notice({ id: 5, read: true }), 4)).toBe('new')
    expect(noticeState(notice({ id: 5, read: false }), 5)).toBe('read')
  })
})

describe('readThrough', () => {
  it('marks up to the newest notice on screen', () => {
    expect(readThrough([notice({ id: 7 }), notice({ id: 9 }), notice({ id: 8 })], 3)).toBe(9)
  })

  it('says nothing when nothing on screen is past the cursor', () => {
    // A reopened inbox must not post the same cursor on every visit.
    expect(readThrough([notice({ id: 7 })], 7)).toBeNull()
    expect(readThrough([], 0)).toBeNull()
  })
})

describe('segments', () => {
  it('draws a command in backticks as code', () => {
    expect(segments('The close did not arrive: `kasl server push --date 2026-09-22` sends it again.')).toEqual([
      { text: 'The close did not arrive: ', code: false },
      { text: 'kasl server push --date 2026-09-22', code: true },
      { text: ' sends it again.', code: false },
    ])
  })

  it('keeps a sentence without backticks whole', () => {
    expect(segments('Nothing had arrived for 13 h.')).toEqual([{ text: 'Nothing had arrived for 13 h.', code: false }])
  })

  it('leaves an unpaired backtick as text', () => {
    // Not a code block that swallows the rest of the sentence.
    expect(segments('one `two` three `four')).toEqual([
      { text: 'one ', code: false },
      { text: 'two', code: true },
      { text: ' three `four', code: false },
    ])
  })
})

describe('screenFor', () => {
  it('points an alert at the day and a privacy change at the manifest', () => {
    expect(screenFor(notice({ kind: 'alert.raised' }))).toBe('/day')
    expect(screenFor(notice({ kind: 'privacy.changed' }))).toBe('/privacy')
    expect(screenFor(notice({ kind: 'agent.issued' }))).toBeNull()
  })
})
