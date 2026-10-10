/**
 * Notes on a day, as the week draws them (ADR 0021).
 *
 * The decisions worth a test live here rather than in the page: which notes
 * belong to which line of the week, who is offered the way to take one back,
 * and which day a link asks to open.
 */

import type { Identity, Note } from '@/lib/api'

/**
 * The notes of a range, by the date they are written on, oldest first within
 * a date - the order the server answers them in, kept rather than re-sorted.
 */
export function notesByDate(notes: Note[]): Map<string, Note[]> {
  const byDate = new Map<string, Note[]>()
  for (const note of notes) {
    const list = byDate.get(note.date)
    if (list) list.push(note)
    else byDate.set(note.date, [note])
  }
  return byDate
}

/**
 * Whether the reader is offered the withdraw button: whoever wrote the note,
 * or an administrator. The server decides the same thing again - this only
 * keeps a button off the screen that would answer "not allowed".
 */
export function mayWithdraw(note: Note, reader: Pick<Identity, 'id' | 'role'>): boolean {
  return reader.role === 'admin' || note.author_id === reader.id
}

/**
 * The day a `?date=` asks for, or null for anything that is not one.
 *
 * A link from a notice opens the week of the day it is about with that day
 * expanded. What arrives in the address bar is whatever somebody typed, so a
 * malformed or impossible date is ignored rather than turned into a week of
 * `NaN`: `2026-02-30` would otherwise roll over to March without a word.
 */
export function askedDay(value: string | null): string | null {
  const parts = value ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(value) : null
  if (!parts) return null
  const [year, month, day] = [Number(parts[1]), Number(parts[2]), Number(parts[3])]
  const date = new Date(year, month - 1, day)
  const real = date.getFullYear() === year && date.getMonth() === month - 1 && date.getDate() === day
  return real ? value : null
}
