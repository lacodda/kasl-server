import { describe, expect, it } from 'vitest'
import type { Note } from '@/lib/api'
import { isoDate, startOfWeek } from '@/lib/day'
import { askedDay, atMidday, mayWithdraw, notesByDate } from '@/lib/notes'

const note = (fields: Partial<Note>): Note => ({
  id: 'n1',
  date: '2026-10-02',
  text: 'Your day off is approved.',
  author_id: 'manager',
  author: 'Priya Raman',
  created_at: '2026-09-30T10:00:00Z',
  ...fields,
})

describe('notesByDate', () => {
  it('files each note under its date and keeps the order it came in', () => {
    const byDate = notesByDate([
      note({ id: 'a', date: '2026-09-29' }),
      note({ id: 'b', date: '2026-10-02' }),
      note({ id: 'c', date: '2026-10-02' }),
    ])
    expect(byDate.get('2026-09-29')?.map((n) => n.id)).toEqual(['a'])
    expect(byDate.get('2026-10-02')?.map((n) => n.id)).toEqual(['b', 'c'])
    expect(byDate.get('2026-10-01')).toBeUndefined()
  })
})

describe('mayWithdraw', () => {
  it('offers the button to the author and to an administrator only', () => {
    expect(mayWithdraw(note({}), { id: 'manager', role: 'manager' })).toBe(true)
    expect(mayWithdraw(note({}), { id: 'someone', role: 'admin' })).toBe(true)
    expect(mayWithdraw(note({}), { id: 'other-manager', role: 'manager' })).toBe(false)
    expect(mayWithdraw(note({}), { id: 'employee', role: 'employee' })).toBe(false)
  })

  it('does not hand an author-less note to whoever has no id either', () => {
    // A note whose author's account is gone: `null` must not equal a reader.
    expect(mayWithdraw(note({ author_id: null }), { id: '', role: 'manager' })).toBe(false)
  })
})

describe('askedDay', () => {
  it('takes a real date', () => {
    expect(askedDay('2026-10-02')).toBe('2026-10-02')
    expect(askedDay('2028-02-29')).toBe('2028-02-29')
  })

  it('ignores what is not one', () => {
    expect(askedDay(null)).toBeNull()
    expect(askedDay('')).toBeNull()
    expect(askedDay('yesterday')).toBeNull()
    expect(askedDay('2026-10-2')).toBeNull()
    // Would roll over to March 2 in `Date`, and open the wrong week silently.
    expect(askedDay('2026-02-30')).toBeNull()
    expect(askedDay('2026-13-01')).toBeNull()
  })

  it('opens the week the day is in', () => {
    // A Friday opens the week that started on its Monday, whatever the zone.
    expect(isoDate(startOfWeek(atMidday('2026-10-02')))).toBe('2026-09-28')
    // A Sunday belongs to the week before, not the one about to start.
    expect(isoDate(startOfWeek(atMidday('2026-10-04')))).toBe('2026-09-28')
  })
})
