import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Coffee, Lock, Plane } from 'lucide-react'
import { PeriodPicker } from '@/components/PeriodPicker'
import { api, type Day, type DaysResponse, type NotStored } from '@/lib/api'
import { bands, clock, duration, isoDate, shiftWeeks, startOfWeek, weekDates, weekdayName } from '@/lib/day'
import { Panel } from '@/components/ui/panel'
import { Progress } from '@/components/ui/progress'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import { Track, type TrackSegment } from '@/components/ui/track'

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
      {/* Stacked on a phone, side by side from `sm`. The week's arrows are a
          row of their own below the title rather than squeezed beside it: at
          320px the title, the dates and three controls on one line leave the
          title two words wide. */}
      <header className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold">{title}</h1>
          {subtitle}
          <p className="mt-1 font-mono text-xs text-faint tabular">
            {from} — {to}
          </p>
        </div>
        <PeriodPicker
          previousLabel={t('myDay.previousWeek')}
          nextLabel={t('myDay.nextWeek')}
          onPrevious={() => goto(-1)}
          onNext={() => goto(1)}
          onNow={() => setMonday(startOfWeek(new Date()))}
        >
          {t('myDay.thisWeek')}
        </PeriodPicker>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {answer && (
        <>
          <WeekTotal answer={answer} />

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

/**
 * The week's worked hours against what it asked for.
 *
 * The two figures sit side by side and the bar divides them - the server
 * answers a pair rather than a percentage, because "32h of 40h" and "80%" are
 * not the same sentence and only one of them survives a part-time week
 * (ADR 0017).
 */
function WeekTotal({ answer }: { answer: DaysResponse }) {
  const { t } = useTranslation()
  // Open days contribute nothing rather than a partial figure: the total says
  // how much work is on the record, and a running day is not on it yet.
  const worked = answer.worked_seconds
  const paused = answer.days.reduce((sum, day) => sum + day.paused_seconds, 0)
  const norm = answer.progress.norm_seconds
  const over = worked - norm

  return (
    <Panel className="space-y-4 p-4 sm:p-5">
      <StatRow>
        <StatTile label={t('myDay.worked')} value={duration(worked)} tone="accent" />
        <StatTile label={t('myDay.norm')} value={norm > 0 ? duration(norm) : '—'} />
        <StatTile label={t('myDay.paused')} value={duration(paused)} />
        <StatTile label={t('myDay.daysRecorded')} value={String(answer.days.length)} />
      </StatRow>

      {norm > 0 ? (
        <Progress
          // Clamped at the norm so the bar stays a bar: a week worked over its
          // norm would otherwise fill past the track and say nothing about how
          // far over it went. The figure beside it is not clamped, and that is
          // where the overtime is stated.
          value={Math.min(worked, norm)}
          max={norm}
          label={t('myDay.progressLabel')}
          // Over the norm is not a warning tone. Nothing on this screen calls
          // a number good or bad - it says what happened, and how much of it
          // was due (ADR 0017).
          tone="accent"
        >
          <span className="font-mono tabular">
            {t('myDay.progress', { worked: duration(worked), norm: duration(norm) })}
            {over > 0 && <span className="ml-2 text-faint">{t('myDay.overNorm', { over: duration(over) })}</span>}
          </span>
        </Progress>
      ) : (
        // A week that owes nothing - a full week of leave, or a rate of zero.
        // Said in words rather than drawn as an empty bar, which would read as
        // "nothing done" instead of "nothing due".
        <p className="text-xs text-faint">{t('myDay.noNorm')}</p>
      )}

      {answer.progress.work_rate !== 1 && (
        // Why this person's norm is not the installation's full week. Without
        // it a half-time week reads as a half-hearted one.
        <p className="text-xs text-faint">{t('myDay.partTime', { rate: answer.progress.work_rate })}</p>
      )}
    </Panel>
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
      <div className="flex items-center gap-3 px-4 py-3.5 opacity-55 sm:gap-4 sm:px-5">
        <DayLabel date={date} weekday={weekday} today={today} />
        <span className="text-sm text-faint">{t('myDay.noData')}</span>
      </div>
    )
  }

  // A day the employee told us they were away. Its own row rather than a bar
  // of nothing: an empty timeline says "worked no hours", and this day was
  // never going to have any (ADR 0017).
  if (day.kind !== 'work') {
    return (
      <div className="flex items-center gap-3 px-4 py-3.5 sm:gap-4 sm:px-5">
        <DayLabel date={date} weekday={weekday} today={today} />
        <span className="inline-flex items-center gap-1.5 text-sm text-dim">
          <Plane className="size-3.5 shrink-0" />
          {t(`myDay.dayKind.${day.kind}`)}
        </span>
      </div>
    )
  }

  const total = (
    <div className="font-mono text-sm tabular">{duration(day.worked_seconds)}</div>
  )
  const tasks = day.tasks.length > 0 && (
    <div className="text-[11px] text-faint">{t('myDay.taskCount', { count: day.tasks.length })}</div>
  )

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-expanded={selected}
      className={`flex w-full cursor-pointer flex-col gap-2 px-4 py-3.5 text-left transition-colors hover:bg-soft sm:flex-row sm:items-center sm:gap-4 sm:px-5 ${
        selected ? 'bg-soft' : ''
      }`}
    >
      {/* On a phone the day's name and its total share the top line, and the
          bar gets the full width underneath. Keeping the desktop's three
          columns would leave the bar about eighty pixels wide, which is not a
          drawing of a day - it is a smudge. */}
      <div className="flex items-baseline justify-between gap-3 sm:contents">
        <DayLabel date={date} weekday={weekday} today={today} />
        <div className="flex items-baseline gap-2 sm:hidden">
          {tasks}
          {total}
        </div>
      </div>

      <div className="min-w-0 flex-1">
        <Timeline day={day} withheld={pausesWithheld} />
        {/* Wrapping rather than one line: "12:04–21:30" and a break count in
            mono at 320px are a few pixels over, and a clipped end time is the
            half of the pair that says whether the day is finished. */}
        <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-faint tabular">
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

      <div className="hidden shrink-0 text-right sm:block">
        {total}
        {tasks && <div className="mt-0.5">{tasks}</div>}
      </div>
    </button>
  )
}

function DayLabel({ date, weekday, today }: { date: string; weekday: string; today: boolean }) {
  return (
    // Its fixed width is what keeps the bars of seven days starting at the
    // same place; on a phone the label is on its own line and the width would
    // only pin it away from the total beside it.
    <div className="shrink-0 sm:w-20">
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
  const drawn = withheld ? null : bands(day)

  if (drawn === null || drawn.bands.length === 0) {
    return (
      <div className="flex h-2.5 items-center">
        <div className="h-2.5 flex-1 rounded-full bg-softer" />
        <span className="ml-3 shrink-0 text-[11px] text-faint">
          {withheld ? t('myDay.timelineWithheld') : t('myDay.timelineEmpty')}
        </span>
      </div>
    )
  }

  const segments: TrackSegment[] = drawn.bands.map((band, index) => ({
    key: String(index),
    start: band.start,
    end: band.end,
    tone: band.paused ? 'idle' : 'accent',
    label: t(`myDay.band.${band.label}`),
  }))

  return (
    <Track
      segments={segments}
      from={drawn.from}
      to={drawn.to}
      // The segments are shapes to a screen reader and their titles are not
      // announced in order, so the bar says in one sentence what it draws.
      label={t('myDay.timelineLabel', {
        from: clock(day.started_at),
        to: day.ended_at ? clock(day.ended_at) : t('myDay.running'),
        worked: duration(day.worked_seconds),
      })}
    />
  )
}

/** The opened day: its pauses and its tasks, or why they are not there. */
function DayDetail({ day, notStored }: { day: Day; notStored: NotStored[] }) {
  const { t } = useTranslation()

  return (
    <div className="grid gap-4 sm:grid-cols-2 sm:gap-5">
      <Panel className="p-4 sm:p-5">
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

      <Panel className="p-4 sm:p-5">
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
