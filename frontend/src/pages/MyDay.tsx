import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronLeft, ChevronRight, Coffee, Lock } from 'lucide-react'
import { api, type Day, type DaysResponse, type NotStored } from '@/lib/api'
import { bands, clock, duration, isoDate, shiftWeeks, startOfWeek, weekDates, weekdayName } from '@/lib/day'
import { Panel } from '@/components/ui/panel'
import { Button } from '@/components/ui/button'

/**
 * The employee's own week: what the server holds about them, in their words.
 */
export function MyDay() {
  const { t } = useTranslation()
  return <WeekView title={t('myDay.title')} load={api.myDays} />
}

/**
 * A week of one person's days, whoever they are.
 *
 * Shared by the personal page and the manager's drill-down: both render the
 * same answer from the server (`/me/days` and `/users/{id}/days` are the same
 * shape by design), and a second copy of this would drift from the first.
 *
 * The screen shows a week at a time and lets one day be opened. Where the
 * installation's privacy level withheld something, it says so in that spot
 * rather than rendering an empty list - which is the whole reason the endpoint
 * reports `not_stored` (ADR 0011).
 */
export function WeekView({
  title,
  subtitle,
  load,
}: {
  title: string
  subtitle?: React.ReactNode
  load: (from: string, to: string) => Promise<DaysResponse>
}) {
  const { t } = useTranslation()
  const [monday, setMonday] = useState(() => startOfWeek(new Date()))
  // The answer carries the range it is for. Clearing it in the effect instead
  // would be a second render pass on every week change - and worse, a late
  // answer for the week just left would land as if it were this one's.
  const [loaded, setLoaded] = useState<{ range: string; answer: DaysResponse | null } | null>(null)
  const [selected, setSelected] = useState<string | null>(null)

  const dates = useMemo(() => weekDates(monday), [monday])
  const from = dates[0]
  const to = dates[6]
  const range = `${from}:${to}`

  useEffect(() => {
    let cancelled = false
    load(from, to)
      .then((value) => {
        if (!cancelled) setLoaded({ range: `${from}:${to}`, answer: value })
      })
      .catch(() => {
        // `null` for this range means it was asked for and failed, which the
        // render tells apart from a range still in flight.
        if (!cancelled) setLoaded({ range: `${from}:${to}`, answer: null })
      })
    return () => {
      cancelled = true
    }
  }, [from, to, load])

  const current = loaded?.range === range ? loaded : null
  const answer = current?.answer ?? null
  const failed = current !== null && current.answer === null

  const goto = useCallback((weeks: number) => {
    setMonday((current) => shiftWeeks(current, weeks))
    // The open day belongs to the week that is leaving.
    setSelected(null)
  }, [])

  const byDate = useMemo(() => new Map(answer?.days.map((day) => [day.date, day]) ?? []), [answer])
  const today = isoDate(new Date())
  const open = selected ? byDate.get(selected) : undefined

  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <header className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold">{title}</h1>
          {subtitle}
          <p className="mt-1 font-mono text-xs text-faint tabular">
            {from} — {to}
          </p>
        </div>
        <div className="flex items-center gap-1.5">
          <Button variant="icon" size="icon-md" aria-label={t('myDay.previousWeek')} onClick={() => goto(-1)}>
            <ChevronLeft />
          </Button>
          <Button size="sm" onClick={() => setMonday(startOfWeek(new Date()))}>
            {t('myDay.thisWeek')}
          </Button>
          <Button variant="icon" size="icon-md" aria-label={t('myDay.nextWeek')} onClick={() => goto(1)}>
            <ChevronRight />
          </Button>
        </div>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {answer && (
        <>
          <WeekTotal days={answer.days} />

          <Panel className="divide-y divide-line">
            {dates.map((date) => (
              <DayRow
                key={date}
                date={date}
                day={byDate.get(date)}
                today={date === today}
                pausesWithheld={answer.not_stored.includes('pauses')}
                selected={date === selected}
                onSelect={() => setSelected(date === selected ? null : date)}
              />
            ))}
          </Panel>

          {open && <DayDetail day={open} notStored={answer.not_stored} />}
        </>
      )}
    </div>
  )
}

/** The week's worked hours, from the days the server answered. */
function WeekTotal({ days }: { days: Day[] }) {
  const { t } = useTranslation()
  // Open days contribute nothing rather than a partial figure: the total says
  // how much work is on the record, and a running day is not on it yet.
  const worked = days.reduce((sum, day) => sum + (day.worked_seconds ?? 0), 0)
  const paused = days.reduce((sum, day) => sum + day.paused_seconds, 0)

  return (
    <Panel className="flex flex-wrap items-baseline gap-x-8 gap-y-3 p-5">
      <Figure label={t('myDay.worked')} value={duration(worked)} accent />
      <Figure label={t('myDay.paused')} value={duration(paused)} />
      <Figure label={t('myDay.daysRecorded')} value={String(days.length)} />
    </Panel>
  )
}

function Figure({ label, value, accent = false }: { label: string; value: string; accent?: boolean }) {
  return (
    <div>
      <div className="text-xs font-medium text-dim">{label}</div>
      <div className={`mt-1 font-mono text-lg tabular ${accent ? 'text-accent-2' : ''}`}>{value}</div>
    </div>
  )
}

/** One day in the week list: its hours, and the timeline of how it went. */
function DayRow({
  date,
  day,
  today,
  pausesWithheld,
  selected,
  onSelect,
}: {
  date: string
  day: Day | undefined
  today: boolean
  pausesWithheld: boolean
  selected: boolean
  onSelect: () => void
}) {
  const { t } = useTranslation()
  const weekday = weekdayName(date)

  if (!day) {
    return (
      <div className="flex items-center gap-4 px-5 py-3.5 opacity-55">
        <DayLabel date={date} weekday={weekday} today={today} />
        <span className="text-sm text-faint">{t('myDay.noData')}</span>
      </div>
    )
  }

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-expanded={selected}
      className={`flex w-full cursor-pointer items-center gap-4 px-5 py-3.5 text-left transition-colors hover:bg-soft ${
        selected ? 'bg-soft' : ''
      }`}
    >
      <DayLabel date={date} weekday={weekday} today={today} />

      <div className="min-w-0 flex-1">
        <Timeline day={day} withheld={pausesWithheld} />
        <div className="mt-1.5 flex items-center gap-3 font-mono text-[11px] text-faint tabular">
          <span>
            {clock(day.started_at)}
            {day.ended_at ? `–${clock(day.ended_at)}` : `–${t('myDay.running')}`}
          </span>
          {day.paused_count > 0 && (
            <span className="inline-flex items-center gap-1">
              <Coffee className="size-3" />
              {day.paused_count} · {duration(day.paused_seconds)}
            </span>
          )}
        </div>
      </div>

      <div className="shrink-0 text-right">
        <div className="font-mono text-sm tabular">{duration(day.worked_seconds)}</div>
        {day.tasks.length > 0 && (
          <div className="mt-0.5 text-[11px] text-faint">{t('myDay.taskCount', { count: day.tasks.length })}</div>
        )}
      </div>
    </button>
  )
}

function DayLabel({ date, weekday, today }: { date: string; weekday: string; today: boolean }) {
  return (
    <div className="w-20 shrink-0">
      <div className={`text-sm font-medium ${today ? 'text-accent-2' : ''}`}>{weekday}</div>
      <div className="font-mono text-[11px] text-faint tabular">{date.slice(5)}</div>
    </div>
  )
}

/**
 * The day drawn as gold worked stretches broken by its pauses - the mockup's
 * signature component, echoing the `ks` mark.
 *
 * Where pauses were not stored the bar is deliberately not drawn: an unbroken
 * gold day is a claim about how the day went, and this installation did not
 * keep what would back it up.
 */
function Timeline({ day, withheld }: { day: Day; withheld: boolean }) {
  const { t } = useTranslation()
  const drawn = withheld ? [] : bands(day)

  if (drawn.length === 0) {
    return (
      <div className="flex h-2.5 items-center">
        <div className="h-2.5 flex-1 rounded-full bg-softer" />
        <span className="ml-3 shrink-0 text-[11px] text-faint">
          {withheld ? t('myDay.timelineWithheld') : t('myDay.timelineEmpty')}
        </span>
      </div>
    )
  }

  return (
    <div className="relative h-2.5 overflow-hidden rounded-full bg-softer">
      {drawn.map((band, index) => (
        <div
          key={index}
          title={t(`myDay.band.${band.label}`)}
          className={`absolute inset-y-0 ${band.paused ? 'bg-line-2' : 'bg-accent'}`}
          style={{ left: `${band.left}%`, width: `${band.width}%` }}
        />
      ))}
    </div>
  )
}

/** The opened day: its pauses and its tasks, or why they are not there. */
function DayDetail({ day, notStored }: { day: Day; notStored: NotStored[] }) {
  const { t } = useTranslation()

  return (
    <div className="grid gap-5 sm:grid-cols-2">
      <Panel className="p-5">
        <h2 className="text-xs font-medium tracking-wide text-dim uppercase">{t('myDay.pauses')}</h2>
        {notStored.includes('pauses') ? (
          <Withheld
            // The count and the total survive even where the rows do not, so
            // the day still adds up: this is not "nothing happened".
            note={t('myDay.pausesWithheld', { count: day.paused_count, total: duration(day.paused_seconds) })}
          />
        ) : day.pauses.length === 0 ? (
          <p className="mt-3 text-sm text-faint">{t('myDay.noPauses')}</p>
        ) : (
          <ul className="mt-3 space-y-2.5">
            {day.pauses.map((pause) => (
              <li key={pause.id} className="flex items-baseline justify-between gap-3">
                <span className="font-mono text-xs tabular">
                  {clock(pause.started_at)}
                  {pause.ended_at && `–${clock(pause.ended_at)}`}
                </span>
                <span className="min-w-0 flex-1 truncate text-right text-sm text-dim">
                  {pause.reason ?? t(pause.manual ? 'myDay.band.break' : 'myDay.band.idle')}
                </span>
                <span className="font-mono text-xs text-faint tabular">{duration(pause.duration_seconds)}</span>
              </li>
            ))}
          </ul>
        )}
      </Panel>

      <Panel className="p-5">
        <h2 className="text-xs font-medium tracking-wide text-dim uppercase">{t('myDay.tasks')}</h2>
        {notStored.includes('tasks') ? (
          <Withheld note={t('myDay.tasksWithheld')} />
        ) : day.tasks.length === 0 ? (
          <p className="mt-3 text-sm text-faint">{t('myDay.noTasks')}</p>
        ) : (
          <ul className="mt-3 space-y-3">
            {day.tasks.map((task) => (
              <li key={task.id}>
                <div className="flex items-baseline justify-between gap-3">
                  <span className="min-w-0 flex-1 truncate text-sm">{task.name}</span>
                  <span className="font-mono text-xs text-accent-2 tabular">{task.completeness}%</span>
                </div>
                {task.comment && <p className="mt-0.5 text-xs text-faint">{task.comment}</p>}
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </div>
  )
}

/** What stands where data would be, when the installation does not keep it. */
function Withheld({ note }: { note: string }) {
  const { t } = useTranslation()
  return (
    <div className="mt-3 space-y-2">
      <p className="flex items-start gap-2 text-sm text-dim">
        <Lock className="mt-0.5 size-3.5 shrink-0 text-faint" />
        <span>{note}</span>
      </p>
      <a href="/privacy" className="inline-block text-xs text-accent-2 underline underline-offset-2">
        {t('myDay.whyNotStored')}
      </a>
    </div>
  )
}
