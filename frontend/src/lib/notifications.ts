/**
 * The inbox's arithmetic: what counts as new, what is over, and how far a
 * visit has read.
 *
 * Here rather than in the page because these are the decisions worth a test.
 * The server keeps the cursor and says what is unread; this side only has to
 * draw it honestly and say, once the list is on screen, how far it got.
 */

import type { Notification } from '@/lib/api'

/** How one notice is drawn. */
export type NoticeState = 'new' | 'read' | 'over'

/**
 * Over outranks unread. A notice whose alert has resolved - the day finally
 * closed - is still a fact ("your manager was told on Monday"), and drawing it
 * as new would ask for attention nothing needs any more. The server's badge
 * leaves it out for the same reason.
 *
 * New is measured against `openedAt`, the read cursor as it stood when the
 * reader opened the screen - not against the server's `read` flag. Opening the
 * screen marks everything read a moment later, and a mark that vanished the
 * instant it was drawn would tell nobody which notices were the new ones.
 */
export function noticeState(notice: Notification, openedAt: number): NoticeState {
  if (notice.withdrawn_at) return 'over'
  return notice.id > openedAt ? 'new' : 'read'
}

/**
 * What to mark read after showing `notices`: the newest id on screen, when it
 * is past the cursor. `null` when there is nothing new to say - an inbox
 * reopened should not post the same cursor again on every visit.
 *
 * The newest *on screen*, not a number from anywhere else: a notice that
 * arrived after the list was fetched has not been seen, and marking past it
 * would hide it from kasl's toast too.
 */
export function readThrough(notices: Notification[], cursor: number): number | null {
  const newest = notices.reduce((max, notice) => Math.max(max, notice.id), 0)
  return newest > cursor ? newest : null
}

/** A piece of a notice's sentence: text, or a command to copy. */
export interface Segment {
  text: string
  code: boolean
}

/**
 * Splits a sentence on backticks, so a command in it - `kasl server push
 * --date 2026-09-22` - is drawn as code, the way the terminal the person will
 * paste it into shows it. An unpaired backtick is left as text rather than
 * swallowing the rest of the sentence into a code block.
 */
export function segments(body: string): Segment[] {
  const parts = body.split('`')
  // An even number of parts means an odd number of backticks: the last one
  // has no partner. Glue it back on as the character it was.
  if (parts.length % 2 === 0) {
    const tail = parts.pop() ?? ''
    parts[parts.length - 1] += '`' + tail
  }
  return parts
    .map((text, index) => ({ text, code: index % 2 === 1 }))
    .filter((segment) => segment.text !== '')
}

/** Where a notice points in this UI, when it has a screen of its own. */
export function screenFor(notice: Notification): '/day' | '/privacy' | null {
  switch (notice.kind) {
    case 'alert.raised':
      return '/day'
    case 'privacy.changed':
      return '/privacy'
    default:
      return null
  }
}
