import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Trash2 } from 'lucide-react'
import { api, type CalendarDay, type CalendarDayKind, type CalendarYear } from '@/lib/api'
import { useSession } from '@/lib/session'
import { weekdayName } from '@/lib/day'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Panel } from '@/components/ui/panel'
import { PeriodPicker } from '@/components/PeriodPicker'
import { DatePicker } from '@/components/ui/date-picker'
import { Select, SelectItem, SelectPopup, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { IsoDate } from '@/components/ui/calendar-math'

/** The three kinds, in the order the list offers them. */
const KINDS: CalendarDayKind[] = ['holiday', 'short_day', 'working_weekend']

/**
 * The production calendar: the dates that differ from the weekday they fall on.
 *
 * Only the exceptions are listed, which is what makes the screen checkable: a
 * year is about a dozen rows, and an administrator can read them against the
 * decree they came from. A grid of 365 days would bury those dozen (ADR 0017).
 *
 * Everyone signed in may read it - which days of the year are worked is not a
 * secret from the people working them - and only an administrator may write.
 * The server enforces both; this screen only keeps the controls out of the way
 * of someone it would refuse.
 */
export function Calendar() {
  const { t } = useTranslation()
  const { user } = useSession()
  const mayEdit = user?.role === 'admin'

  const [year, setYear] = useState(() => new Date().getFullYear())
  // One piece of state for the whole screen, keyed by the year it belongs to.
  //
  // The year, what the server answered and what is being edited are one fact
  // with three parts: a draft from last year must never be saved into this
  // one. Three separate states would have to be kept in step by an effect, and
  // an effect that calls `setState` is a cascade of renders where the stale
  // value is briefly on screen.
  const [state, setState] = useState<{
    year: number
    answer: CalendarYear | null
    draft: CalendarDay[]
    saved: boolean
  } | null>(null)
  const [saving, setSaving] = useState(false)

  const load = useCallback((forYear: number) => {
    let cancelled = false
    api
      .calendar(forYear)
      .then((answer) => {
        if (!cancelled) setState({ year: forYear, answer, draft: answer.days, saved: false })
      })
      .catch(() => {
        if (!cancelled) setState({ year: forYear, answer: null, draft: [], saved: false })
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => load(year), [year, load])

  const current = state?.year === year ? state : null
  const answer = current?.answer ?? null
  const failed = current !== null && current.answer === null

  const days = useMemo(() => current?.draft ?? [], [current])
  // Sorted for the screen only: the server stores a set, and the order a row
  // was typed in is not a fact about the year.
  const sorted = useMemo(() => [...days].sort((a, b) => a.date.localeCompare(b.date)), [days])

  /** Replaces the draft, and drops the "saved" mark the moment it is edited. */
  const edit = (draft: CalendarDay[]) => setState((value) => (value ? { ...value, draft, saved: false } : value))

  const save = async () => {
    setSaving(true)
    try {
      const written = await api.putCalendar(year, sorted)
      setState({ year, answer: written, draft: written.days, saved: true })
    } catch {
      // A failed save leaves the draft alone rather than pretending: what the
      // administrator typed is still the thing they meant to enter.
      setState((value) => (value ? { ...value, saved: false } : value))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold">{t('calendar.title')}</h1>
          <p className="mt-1 text-sm text-dim">{t('calendar.subtitle')}</p>
          <p className="mt-1 font-mono text-xs text-faint tabular">{year}</p>
        </div>
        <PeriodPicker
          previousLabel={t('calendar.previousYear')}
          nextLabel={t('calendar.nextYear')}
          onPrevious={() => setYear((value) => value - 1)}
          onNext={() => setYear((value) => value + 1)}
          onNow={() => setYear(new Date().getFullYear())}
        >
          {t('calendar.thisYear')}
        </PeriodPicker>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {answer && (
        <>
          <Panel className="p-4 sm:p-5">
            <FullDay hours={answer.standard_hours} mayEdit={mayEdit} onSaved={() => load(year)} />
          </Panel>

          <Panel className="divide-y divide-line">
            {sorted.length === 0 ? (
              // What an empty year means, in words. A blank panel would leave
              // the reader to guess whether the calendar is empty or the
              // screen failed to draw it.
              <p className="px-4 py-5 text-sm text-faint sm:px-5">{t('calendar.empty')}</p>
            ) : (
              sorted.map((day) => (
                <DayRow
                  key={day.date}
                  day={day}
                  mayEdit={mayEdit}
                  onChange={(next) => edit(days.map((row) => (row.date === day.date ? next : row)))}
                  onRemove={() => edit(days.filter((row) => row.date !== day.date))}
                />
              ))
            )}
          </Panel>

          {mayEdit ? (
            <div className="flex flex-wrap items-center gap-3">
              <AddDay
                year={year}
                taken={days.map((day) => day.date)}
                onAdd={(day) => edit([...days, day])}
              />
              <Button onClick={() => void save()} disabled={saving}>
                {saving ? t('calendar.saving') : t('calendar.save')}
              </Button>
              {current?.saved && <span className="text-xs text-good">{t('calendar.saved')}</span>}
            </div>
          ) : (
            <p className="text-xs text-faint">{t('calendar.readOnly')}</p>
          )}
        </>
      )}
    </div>
  )
}

/** The installation's full day: the figure every norm is computed from. */
function FullDay({ hours, mayEdit, onSaved }: { hours: number; mayEdit: boolean; onSaved: () => void }) {
  const { t } = useTranslation()
  // What the administrator typed, or nothing yet - in which case the server's
  // figure is shown. Kept this way round rather than seeded from the prop and
  // re-synchronised in an effect: the server's answer has to win when another
  // administrator moves it, and an effect that calls `setState` would put the
  // stale number on screen for a render on the way there.
  const [typed, setTyped] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const value = typed ?? String(hours)
  const dirty = Number(value) !== hours && Number(value) > 0

  return (
    <div className="flex flex-wrap items-center gap-3">
      <label className="text-xs font-medium text-dim" htmlFor="standard-hours">
        {t('calendar.standardHours')}
      </label>
      <Input
        id="standard-hours"
        type="number"
        min="0.5"
        max="24"
        step="0.5"
        value={value}
        disabled={!mayEdit}
        onChange={(event) => setTyped(event.target.value)}
        className="w-24"
      />
      <span className="text-xs text-faint">{t('calendar.hoursUnit')}</span>
      {mayEdit && dirty && (
        <Button
          size="sm"
          disabled={saving}
          onClick={() => {
            setSaving(true)
            api
              .putStandardHours(Number(value))
              .then(() => {
                // Back to showing the server's figure, which the reload is
                // about to bring.
                setTyped(null)
                onSaved()
              })
              .catch(() => setTyped(null))
              .finally(() => setSaving(false))
          }}
        >
          {saving ? t('calendar.saving') : t('calendar.save')}
        </Button>
      )}
    </div>
  )
}

/** One dated exception. */
function DayRow({
  day,
  mayEdit,
  onChange,
  onRemove,
}: {
  day: CalendarDay
  mayEdit: boolean
  onChange: (next: CalendarDay) => void
  onRemove: () => void
}) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-col gap-2 px-4 py-3 sm:flex-row sm:items-center sm:gap-4 sm:px-5">
      <div className="shrink-0 sm:w-32">
        <div className="font-mono text-sm tabular">{day.date}</div>
        <div className="text-[11px] text-faint">{weekdayName(day.date)}</div>
      </div>

      <div className="min-w-0 flex-1">
        {mayEdit ? (
          <Select
            value={day.kind}
            onValueChange={(kind) => onChange({ ...day, kind: kind as CalendarDayKind })}
            items={Object.fromEntries(KINDS.map((kind) => [kind, t(`calendar.kinds.${kind}`)]))}
          >
            <SelectTrigger size="sm" className="w-full sm:w-52" aria-label={t('calendar.kind')}>
              <SelectValue />
            </SelectTrigger>
            <SelectPopup>
              {KINDS.map((kind) => (
                <SelectItem key={kind} value={kind}>
                  {t(`calendar.kinds.${kind}`)}
                </SelectItem>
              ))}
            </SelectPopup>
          </Select>
        ) : (
          <div className="text-sm">{t(`calendar.kinds.${day.kind}`)}</div>
        )}
        {/* What the kind does to the day's hours, in the words of the thing
            itself. A reader entering a calendar should not have to remember
            which of the three subtracts an hour and which adds a day. */}
        <p className="mt-0.5 text-[11px] text-faint">{t(`calendar.kindDetail.${day.kind}`)}</p>
      </div>

      <div className="flex min-w-0 items-center gap-2 sm:w-64">
        {mayEdit ? (
          <Input
            value={day.note ?? ''}
            placeholder={t('calendar.notePlaceholder')}
            aria-label={t('calendar.note')}
            onChange={(event) => onChange({ ...day, note: event.target.value || null })}
          />
        ) : (
          day.note && <span className="truncate text-sm text-dim">{day.note}</span>
        )}
        {mayEdit && (
          <Button variant="ghost" size="sm" onClick={onRemove} aria-label={t('calendar.remove')} className="shrink-0">
            <Trash2 className="size-4" />
          </Button>
        )}
      </div>
    </div>
  )
}

/** The control that adds a date to the year. */
function AddDay({ year, taken, onAdd }: { year: number; taken: string[]; onAdd: (day: CalendarDay) => void }) {
  const { t } = useTranslation()
  const [date, setDate] = useState('')

  const inYear = date.startsWith(`${year}-`)
  // A date already in the list is refused here rather than at the server: the
  // year is a set, and two rows for one date is the typo the primary key
  // catches after a round trip.
  const usable = inYear && !taken.includes(date)

  return (
    <div className="flex items-center gap-2">
      {/* Not a native `<input type="date">`. That control takes its format and
          its placeholder from the browser's own language, not the document's,
          so on a non-English Windows it renders that locale's placeholder
          inside an English product - seen on the demo, and not something the
          page can override. The picker spells the date out and asks the
          product for its words. */}
      <DatePicker
        value={(date || undefined) as IsoDate | undefined}
        onValueChange={(value) => setDate(value)}
        min={`${year}-01-01` as IsoDate}
        max={`${year}-12-31` as IsoDate}
        placeholder={t('calendar.pickDate')}
        previousMonthLabel={t('calendar.previousMonth')}
        nextMonthLabel={t('calendar.nextMonth')}
        locale="en"
        aria-label={t('calendar.date')}
        className="w-56"
      />
      <Button
        size="sm"
        disabled={!usable}
        onClick={() => {
          onAdd({ date, kind: 'holiday', note: null })
          setDate('')
        }}
      >
        {t('calendar.add')}
      </Button>
    </div>
  )
}
