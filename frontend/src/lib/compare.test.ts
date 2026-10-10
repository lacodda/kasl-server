import { describe, expect, it } from 'vitest'
import type { Member } from '@/lib/api'
import { compareMembers, departmentTotals, points, sortCompared, totals } from '@/lib/compare'

const HOUR = 3600

function member(overrides: Partial<Member> & { id: string }): Member {
  return {
    display_name: overrides.id,
    email: `${overrides.id}@example.test`,
    department: null,
    active: true,
    days_recorded: 5,
    worked_seconds: 40 * HOUR,
    paused_seconds: 0,
    last_day: null,
    day_open: false,
    last_seen_at: null,
    agents: 1,
    work_rate: 1,
    norm_seconds: 40 * HOUR,
    days_away: 0,
    ...overrides,
    // What has come due follows the norm unless a test says otherwise: most
    // fixtures are a period that is over.
    due_seconds: overrides.due_seconds ?? overrides.norm_seconds ?? 40 * HOUR,
  }
}

describe('compareMembers', () => {
  it('measures each person against their own norm', () => {
    const [full, half] = compareMembers(
      [member({ id: 'full' }), member({ id: 'half', worked_seconds: 20 * HOUR, norm_seconds: 20 * HOUR, work_rate: 0.5 })],
      null,
    )
    // Half the hours, the whole of what was owed: the same share. This is what
    // keeps the table from being a scoreboard.
    expect(full?.share).toBe(1)
    expect(half?.share).toBe(1)
  })

  it('compares a person with themselves in the period before', () => {
    const [row] = compareMembers([member({ id: 'a', worked_seconds: 36 * HOUR })], [member({ id: 'a', worked_seconds: 30 * HOUR })])
    expect(row?.share).toBe(0.9)
    expect(row?.change).toBe(15)
  })

  it('measures a period still running against what has come due', () => {
    // The 10th of a month: 160 hours owed in all, 56 due so far, 56 worked.
    const [row] = compareMembers([member({ id: 'a', worked_seconds: 56 * HOUR, norm_seconds: 160 * HOUR, due_seconds: 56 * HOUR })], null)
    expect(row?.share).toBe(1)
  })

  it('says nothing where there is nothing to compare', () => {
    const rows = compareMembers(
      [
        member({ id: 'new' }),
        member({ id: 'on-leave', norm_seconds: 0, worked_seconds: 0 }),
        member({ id: 'no-agent', agents: 0, worked_seconds: 0 }),
        member({ id: 'back' }),
      ],
      [member({ id: 'back', norm_seconds: 0, worked_seconds: 0 })],
    )
    const byId = new Map(rows.map((row) => [row.member.id, row]))
    expect(byId.get('new')!.change, 'not in the period before').toBeNull()
    expect(byId.get('on-leave')!.share, 'nothing was owed').toBeNull()
    expect(byId.get('no-agent')!.share, 'nothing is measured').toBeNull()
    expect(byId.get('back')!.share).toBe(1)
    expect(byId.get('back')!.change, 'the period before owed nothing').toBeNull()
  })
})

describe('sortCompared', () => {
  const rows = compareMembers(
    [
      member({ id: 'carol', worked_seconds: 40 * HOUR }),
      member({ id: 'alice', worked_seconds: 20 * HOUR }),
      member({ id: 'bob', agents: 0 }),
      member({ id: 'dave', worked_seconds: 30 * HOUR }),
    ],
    [member({ id: 'carol', worked_seconds: 20 * HOUR }), member({ id: 'dave', worked_seconds: 40 * HOUR })],
  )
  const ids = (key: 'name' | 'share' | 'change') => sortCompared(rows, key).map((row) => row.member.id)

  it('by name', () => {
    expect(ids('name')).toEqual(['alice', 'bob', 'carol', 'dave'])
  })

  it('by share, the furthest short first and the unmeasured last', () => {
    expect(ids('share')).toEqual(['alice', 'dave', 'carol', 'bob'])
  })

  it('by change, the furthest fallen first and the incomparable last, by name', () => {
    expect(ids('change')).toEqual(['dave', 'carol', 'alice', 'bob'])
  })
})

describe('totals', () => {
  it('counts only the people the server measures towards the share', () => {
    const team = totals([member({ id: 'a', worked_seconds: 30 * HOUR }), member({ id: 'admin', agents: 0, worked_seconds: 0 })])
    expect(team.people).toBe(2)
    expect(team.due_seconds).toBe(40 * HOUR)
    expect(team.share).toBe(0.75)
  })

  it('has no share where nothing was owed', () => {
    expect(totals([member({ id: 'a', norm_seconds: 0 })]).share).toBeNull()
    expect(totals([]).share).toBeNull()
  })
})

describe('departmentTotals', () => {
  it('groups by department, alphabetically, with nobody-in-one last', () => {
    const groups = departmentTotals(
      [
        member({ id: 'a', department: 'Engineering', worked_seconds: 36 * HOUR }),
        member({ id: 'b', department: null }),
        member({ id: 'c', department: 'Design', worked_seconds: 20 * HOUR }),
        member({ id: 'd', department: 'Engineering', worked_seconds: 36 * HOUR }),
      ],
      [member({ id: 'a', department: 'Engineering', worked_seconds: 40 * HOUR }), member({ id: 'd', department: 'Engineering' })],
    )
    expect(groups.map((group) => group.name)).toEqual(['Design', 'Engineering', null])
    expect(groups[1]).toMatchObject({ name: 'Engineering', people: 2, share: 0.9, change: -10 })
    expect(groups[0]?.change, 'Design was nobody last period').toBeNull()
  })
})

describe('points', () => {
  it('signs a change, with a real minus', () => {
    expect(points(4)).toBe('+4 pts')
    expect(points(-3)).toBe('−3 pts')
    expect(points(0)).toBe('0 pts')
  })
})
