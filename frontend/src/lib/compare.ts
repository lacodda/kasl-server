/**
 * Comparing people and periods on the team screen (ADR 0023).
 *
 * **People are compared with their own norm, never with each other's hours.**
 * The share of norm is already adjusted for a part-time rate and for leave
 * (ADR 0017), so somebody on half time who worked their half reads as 100%,
 * like a full-timer who worked a full week. A sort by raw hours would be the
 * scoreboard this product does not draw (ADR 0016) - a full-timer always
 * ahead of a half-timer, somebody back from holiday always last.
 *
 * **Periods are compared as shares too**: September has more working days
 * than August, and comparing their hours compares calendars.
 *
 * **A share is of the norm that has come due**, not of the whole period's: on
 * the 10th, somebody exactly on track has worked a third of the month's norm
 * and all of what was due by then. For a period that is over the two are the
 * same.
 *
 * Only people the server measures count towards a share - somebody with no
 * agent installed owes hours nothing will ever report, and "0% of norm" would
 * be a claim about them rather than about the installation.
 */

import type { Member } from '@/lib/api'
import { shareChange, shareOfNorm } from '@/lib/period'

/** A person's row: their figures, their share, and how it moved. */
export interface Compared {
  member: Member
  /** Of their own norm, or `null` where nothing was owed or nothing is measured. */
  share: number | null
  /** Percentage points against the period before, or `null`. */
  change: number | null
}

/** Whether the server measures this person at all. */
export function measured(member: Member): boolean {
  return member.agents > 0
}

function shareOf(member: Member | undefined): number | null {
  if (!member || !measured(member)) return null
  return shareOfNorm(member.worked_seconds, member.due_seconds)
}

/**
 * Each person of this period beside themselves in the one before.
 *
 * Matched by id. Somebody new this period has nothing to compare with, and
 * says nothing rather than a jump from zero.
 */
export function compareMembers(current: Member[], previous: Member[] | null): Compared[] {
  const before = new Map(previous?.map((member) => [member.id, member]) ?? [])
  return current.map((member) => {
    const share = shareOf(member)
    return { member, share, change: shareChange(share, shareOf(before.get(member.id))) }
  })
}

export type SortKey = 'name' | 'share' | 'change'

/** The orders the table offers. Deliberately no order by hours. */
export const SORT_KEYS: readonly SortKey[] = ['name', 'share', 'change']

/**
 * The rows in the order asked for.
 *
 * Share and change put who to look at first: the furthest short of their norm,
 * the furthest fallen. Somebody with nothing to compare goes last whichever way
 * - a missing figure is not the lowest one. Ties fall back to the name, so the
 * order is the same on every load.
 */
export function sortCompared(rows: Compared[], key: SortKey): Compared[] {
  const byName = (a: Compared, b: Compared) =>
    a.member.display_name.localeCompare(b.member.display_name) || a.member.email.localeCompare(b.member.email)
  if (key === 'name') return [...rows].sort(byName)

  const figure = (row: Compared) => (key === 'share' ? row.share : row.change)
  return [...rows].sort((a, b) => {
    const left = figure(a)
    const right = figure(b)
    if (left === null && right === null) return byName(a, b)
    if (left === null) return 1
    if (right === null) return -1
    return left - right || byName(a, b)
  })
}

/** The figures of a group of people: a department, or the whole team. */
export interface Totals {
  people: number
  worked_seconds: number
  /** The norm that has come due, across the people measured. */
  due_seconds: number
  share: number | null
}

/** What a group comes to, counting only the people the server measures. */
export function totals(members: Member[]): Totals {
  const counted = members.filter(measured)
  const worked = counted.reduce((sum, member) => sum + member.worked_seconds, 0)
  const due = counted.reduce((sum, member) => sum + member.due_seconds, 0)
  return { people: members.length, worked_seconds: worked, due_seconds: due, share: shareOfNorm(worked, due) }
}

/** One department's period beside its period before. */
export interface DepartmentTotals extends Totals {
  /** `null` for the people in no department. */
  name: string | null
  change: number | null
}

/**
 * The period by department, alphabetically, the people in none last.
 *
 * A department is compared with itself in the period before - by its own
 * people then, not by who is in it now, so somebody who moved departments
 * counts where they worked.
 */
export function departmentTotals(current: Member[], previous: Member[] | null): DepartmentTotals[] {
  const names = [...new Set(current.map((member) => member.department))]
  return names
    .sort((a, b) => (a === null ? 1 : b === null ? -1 : a.localeCompare(b)))
    .map((name) => {
      const now = totals(current.filter((member) => member.department === name))
      const then = previous ? totals(previous.filter((member) => member.department === name)) : null
      return { name, ...now, change: shareChange(now.share, then?.share ?? null) }
    })
}

/** A change in points as `+4 pts` or `−3 pts`, with a real minus sign. */
export function points(change: number): string {
  if (change > 0) return `+${change} pts`
  if (change < 0) return `−${Math.abs(change)} pts`
  return '0 pts'
}
