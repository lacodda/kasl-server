import { describe, expect, it } from 'vitest'
import type { Day, Report } from '@/lib/api'
import { movedHours, ownAction, review, statusLook } from '@/lib/reports'

const report = (fields: Partial<Report>): Report => ({
  id: 'r1',
  user_id: 'employee',
  date: '2026-10-02',
  status: 'submitted',
  submitted_at: '2026-10-02T21:00:00Z',
  kind: 'work',
  started_at: '2026-10-02T12:00:00Z',
  ended_at: '2026-10-02T20:30:00Z',
  worked_seconds: 27_000,
  reviewed_at: null,
  reviewer_id: null,
  reviewer: null,
  reason: null,
  ...fields,
})

const day = (fields: Partial<Day>): Day => ({
  date: '2026-10-02',
  kind: 'work',
  started_at: '2026-10-02T12:00:00Z',
  ended_at: '2026-10-02T20:30:00Z',
  worked_seconds: 27_000,
  paused_count: 1,
  paused_seconds: 3_600,
  pauses: [],
  tasks: [],
  norm_seconds: 28_800,
  report: null,
  ...fields,
})

describe('statusLook', () => {
  it('says "waiting" only where somebody is going to answer', () => {
    expect(statusLook('submitted', true).label).toBe('reports.status.waiting')
    expect(statusLook('submitted', false).label).toBe('reports.status.reported')
  })

  it('gives every status a mark of its own, not only a colour', () => {
    const marks = (['submitted', 'approved', 'returned', 'changed'] as const).map((status) => statusLook(status, true).icon)
    expect(new Set(marks).size).toBe(marks.length)
  })

  it('draws what asks the person to act in the warning role', () => {
    expect(statusLook('returned', true).tone).toBe('warn')
    expect(statusLook('changed', true).tone).toBe('warn')
    expect(statusLook('approved', true).tone).toBe('good')
  })
})

describe('ownAction', () => {
  it('offers to report a finished day where days are approved', () => {
    expect(ownAction(day({}), true)).toBe('report')
  })

  it('offers nothing where nobody approves days', () => {
    // A button whose effect nobody sees is a promise the screen does not keep.
    expect(ownAction(day({}), false)).toBeNull()
  })

  it('offers nothing on a day still open', () => {
    expect(ownAction(day({ ended_at: null, worked_seconds: null }), true)).toBeNull()
  })

  it('offers it again once the report no longer stands for the day', () => {
    expect(ownAction(day({ report: report({ status: 'returned' }) }), true)).toBe('again')
    expect(ownAction(day({ report: report({ status: 'changed' }) }), true)).toBe('again')
    expect(ownAction(day({ report: report({ status: 'submitted' }) }), true)).toBeNull()
    expect(ownAction(day({ report: report({ status: 'approved' }) }), true)).toBeNull()
  })

  it('offers a day of leave too: it is on the timesheet like any other', () => {
    expect(ownAction(day({ kind: 'vacation', worked_seconds: 0 }), true)).toBe('report')
  })
})

describe('review', () => {
  it('approves and sends back a waiting report of somebody else', () => {
    expect(review(report({}), true, 'manager')).toEqual({ approve: true, sendBack: true })
  })

  it('can still send back what was approved, and not approve it twice', () => {
    expect(review(report({ status: 'approved' }), true, 'manager')).toEqual({ approve: false, sendBack: true })
  })

  it('leaves a returned or changed report to its person', () => {
    expect(review(report({ status: 'returned' }), true, 'manager')).toEqual({ approve: false, sendBack: false })
    expect(review(report({ status: 'changed' }), true, 'manager')).toEqual({ approve: false, sendBack: false })
  })

  it('offers nobody an answer on their own day', () => {
    expect(review(report({ user_id: 'manager' }), true, 'manager')).toEqual({ approve: false, sendBack: false })
  })

  it('offers nothing where nobody approves days', () => {
    expect(review(report({}), false, 'manager')).toEqual({ approve: false, sendBack: false })
  })
})

describe('movedHours', () => {
  it('names both figures when the hours moved', () => {
    const moved = movedHours(day({ worked_seconds: 30_600 }), report({ status: 'changed' }))
    expect(moved).toEqual({ reported: 27_000, now: 30_600 })
  })

  it('says nothing when only the start and end moved', () => {
    expect(movedHours(day({}), report({ status: 'changed' }))).toBeNull()
  })

  it('says nothing about a report that still stands', () => {
    expect(movedHours(day({ worked_seconds: 30_600 }), report({ status: 'submitted' }))).toBeNull()
  })

  it('carries a reopened day as having no total', () => {
    expect(movedHours(day({ ended_at: null, worked_seconds: null }), report({ status: 'changed' }))).toEqual({ reported: 27_000, now: null })
  })
})
