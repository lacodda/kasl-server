/**
 * Reports and their approval, as the screens draw them (ADR 0022).
 *
 * The decisions worth a test live here rather than in the pages: what a day's
 * report is called, what the person may do about it, and what a manager may.
 * The server decides all of it again - this side only keeps buttons off the
 * screen that would be refused, and says in words what a status means.
 */

import type { Day, Report, ReportStatus } from '@/lib/api'

/** How a status is drawn: its words, its colour role, its mark. */
export interface StatusLook {
  /** The translation key. */
  label: string
  tone: 'good' | 'warn' | 'dim'
  icon: 'approved' | 'waiting' | 'reported' | 'returned' | 'changed'
}

/**
 * The words and the mark of a status.
 *
 * `submitted` reads differently by whether anybody approves days here: with
 * approval on it is waiting for a manager, and saying "reported" would leave
 * the person wondering whether anything is still to happen; with it off,
 * "waiting" would promise an answer nobody is going to give.
 *
 * Every status carries a mark beside its colour. This product's accent is a
 * gold 3.7 ΔE from `warn`, so a status told apart by hue alone is told apart
 * by nothing.
 */
export function statusLook(status: ReportStatus, approval: boolean): StatusLook {
  switch (status) {
    case 'approved':
      return { label: 'reports.status.approved', tone: 'good', icon: 'approved' }
    case 'returned':
      return { label: 'reports.status.returned', tone: 'warn', icon: 'returned' }
    case 'changed':
      return { label: 'reports.status.changed', tone: 'warn', icon: 'changed' }
    case 'submitted':
      return approval
        ? { label: 'reports.status.waiting', tone: 'dim', icon: 'waiting' }
        : { label: 'reports.status.reported', tone: 'dim', icon: 'reported' }
  }
}

/** What the person may do about one of their own days. */
export type OwnAction = 'report' | 'again' | null

/**
 * Whether the person is offered to report a day, and in which words.
 *
 * Only where days are approved: off, a report changes nothing anybody sees,
 * and a button that does nothing visible is a promise the screen does not
 * keep. kasl still reports days there - `report --send` closes the day either
 * way - and those reports are shown.
 *
 * Only a finished day: an open one has no total to put a name to, and the
 * server refuses it. A day already reported is offered again once its report
 * stopped standing for it - sent back, or the day moved since.
 */
export function ownAction(day: Day, approval: boolean): OwnAction {
  if (!approval || day.ended_at === null) return null
  if (!day.report) return 'report'
  return day.report.status === 'returned' || day.report.status === 'changed' ? 'again' : null
}

/** What a manager may do with a report on somebody's day. */
export interface Review {
  approve: boolean
  sendBack: boolean
}

/**
 * The answers a reader is offered on a report, on the drill-down.
 *
 * Nothing on one's own day: nobody approves or returns their own, and the
 * server would refuse it. A report waiting is approved or sent back; one
 * approved can still be sent back - a manager who notices on Monday what they
 * approved on Friday has to be able to say so. One returned or changed waits
 * for its person, not for a manager.
 */
export function review(report: Report, approval: boolean, readerId: string): Review {
  if (!approval || report.user_id === readerId) return { approve: false, sendBack: false }
  return {
    approve: report.status === 'submitted',
    sendBack: report.status === 'submitted' || report.status === 'approved',
  }
}

/**
 * Whether the day as it is now differs in hours from what was reported, so a
 * screen can say both figures rather than only that something moved. Null when
 * the hours are the same - the day may have moved its start or end without
 * changing its total, and the status alone says that.
 */
export function movedHours(day: Day, report: Report): { reported: number; now: number | null } | null {
  if (report.status !== 'changed') return null
  if (day.worked_seconds === report.worked_seconds) return null
  return { reported: report.worked_seconds, now: day.worked_seconds }
}

/** The longest reason the server takes, in characters. Mirrors `reports::MAX_REASON_CHARS`. */
export const MAX_REASON_CHARS = 1000
