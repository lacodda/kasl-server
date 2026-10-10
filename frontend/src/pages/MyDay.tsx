import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { useSearchParams } from 'react-router'
import { useTranslation } from 'react-i18next'
import { CheckCheck, Coffee, Hourglass, Lock, MessageSquareText, Plane, RefreshCcw, Send, Undo2 } from 'lucide-react'
import { ExportMenu } from '@/components/ExportMenu'
import { PeriodControls, PeriodRange } from '@/components/PeriodControls'
import { api, ApiError, type Day, type DaysResponse, type Note, type NotStored, type Report } from '@/lib/api'
import { points } from '@/lib/compare'
import { bands, clock, duration, isoDate, moment, weekdayName } from '@/lib/day'
import { askedDay, mayWithdraw, notesByDate } from '@/lib/notes'
import { percent, periodDates, shareChange, shareOfNorm, shiftPeriod, type Period, type PeriodUnit } from '@/lib/period'
import { MAX_REASON_CHARS, movedHours, ownAction, review, statusLook, type StatusLook } from '@/lib/reports'
import { useSession } from '@/lib/session'
import { usePeriod } from '@/lib/use-period'
import { Button } from '@/components/ui/button'
import { Panel } from '@/components/ui/panel'
import { Progress } from '@/components/ui/progress'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import { Textarea } from '@/components/ui/textarea'
import { Track, type TrackSegment } from '@/components/ui/track'

/**
 * The employee's own days: what the server holds about them, in their words.
 */
export function MyDay() {
  const { t } = useTranslation()
  const title = useCallback((unit: PeriodUnit) => t(`myDay.title.${unit}`), [t])
  return <WeekView title={title} load={api.myDays} exportable />
}

/**
 * A period of one person's days, whoever they are.
 *
 * Shared by the personal page and the manager's drill-down: both render the
 * same answer from the server (`/me/days` and `/users/{id}/days` are the same
 * shape by design), and a second copy of this would drift from the first.
 *
 * The screen shows a day, a week or a month at a time (ADR 0023) and lets one
 * day be opened. Where the installation's privacy level withheld something,
 * it says so in that spot rather than rendering an empty list - which is the
 * whole reason the endpoint reports `not_stored` (ADR 0011).
 *
 * A manager's notes sit on the line of the day they are written on, whether or
 * not that day was ever worked (ADR 0021). `?date=` opens the period around
 * that day with it expanded, which is where a notice about one day points.
 *
 * A day's report stands on its line too (ADR 0022): the person reports a
 * finished day from here, and on the drill-down its manager answers it.
 */
export function WeekView({
  title,
  subtitle,
  load,
  writeFor,
  exportable = false,
}: {
  /** The heading, by the unit on screen: "My week", "My month". */
  title: (unit: PeriodUnit) => string
  subtitle?: ReactNode
  load: (from: string, to: string) => Promise<DaysResponse>
  /**
   * Whose days these are, when the reader may write notes on them: the
   * drill-down. The personal page passes nothing - nobody writes on their own
   * day, and the server would refuse it.
   */
  writeFor?: string
  /**
   * Whether the period can be downloaded from here: the personal page. The
   * drill-down's person is in the team's export, which is downloaded from
   * the team screen.
   */
  exportable?: boolean
}) {
  const { t } = useTranslation()
  const [params] = useSearchParams()
  // Read once, on arrival: the link asks for a day, and from then on the
  // arrows are the reader's.
  const [asked] = useState(() => askedDay(params.get('date')))
  const [period, setPeriod] = usePeriod()
  // The answer carries the range it is for. Clearing it in the effect instead
  // would be a second render pass on every period change - and worse, a late
  // answer for the period just left would land as if it were this one's. The
  // period before rides along for the comparison.
  const [loaded, setLoaded] = useState<{ range: string; answer: DaysResponse | null; before: DaysResponse | null } | null>(null)
  const [selected, setSelected] = useState<string | null>(asked)
  // Bumped when a note is written or withdrawn, so the period is asked for
  // again and shows what the server now holds rather than a local guess.
  const [revision, setRevision] = useState(0)
  const reload = useCallback(() => setRevision((current) => current + 1), [])

  // Whole periods: what a period still running is measured against is the
  // norm that has come due, answered beside the whole norm (ADR 0023).
  const { from, to } = period
  const { from: beforeFrom, to: beforeTo } = shiftPeriod(period, -1)
  const range = `${from}:${to}`
  const dates = useMemo(() => periodDates({ unit: 'day', from, to }), [from, to])
  const today = isoDate(new Date())

  useEffect(() => {
    let cancelled = false
    const key = `${from}:${to}`
    Promise.all([
      load(from, to),
      // A failed comparison costs the comparison, not the page.
      load(beforeFrom, beforeTo).catch(() => null),
    ])
      .then(([answer, before]) => {
        if (!cancelled) setLoaded({ range: key, answer, before })
      })
      .catch(() => {
        // `null` for this range means it was asked for and failed, which the
        // render tells apart from a range still in flight.
        if (!cancelled) setLoaded({ range: key, answer: null, before: null })
      })
    return () => {
      cancelled = true
    }
  }, [from, to, beforeFrom, beforeTo, load, revision])

  const current = loaded?.range === range ? loaded : null
  const answer = current?.answer ?? null
  const failed = current !== null && current.answer === null

  const goto = useCallback(
    (next: Period) => {
      setPeriod(next)
      // The open day belongs to the period that is leaving.
      setSelected(null)
    },
    [setPeriod],
  )

  const byDate = useMemo(() => new Map(answer?.days.map((day) => [day.date, day]) ?? []), [answer])
  const notes = useMemo(() => notesByDate(answer?.notes ?? []), [answer])

  return (
    <div className="mx-auto max-w-3xl space-y-5">
      {/* Stacked on a phone, side by side from `sm`. The period's controls are
          a row of their own below the title rather than squeezed beside it: at
          320px the title, the dates and the controls on one line leave the
          title two words wide. */}
      <header className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold">{title(period.unit)}</h1>
          {subtitle}
          <PeriodRange period={period} />
        </div>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
          <PeriodControls period={period} onChange={goto} />
          {exportable && <ExportMenu subject="me" period={period} />}
        </div>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {answer && (
        <>
          <PeriodTotal answer={answer} before={current?.before ?? null} unit={period.unit} />

          <Panel className="divide-y divide-line">
            {dates.map((date) => (
              <DayRow
                key={date}
                date={date}
                day={byDate.get(date)}
                notes={notes.get(date) ?? []}
                today={date === today}
                pausesWithheld={answer.not_stored.includes('pauses')}
                writable={writeFor !== undefined}
                approval={answer.day_approval}
                selected={date === selected}
                onSelect={() => setSelected(date === selected ? null : date)}
              />
            ))}
          </Panel>

          {selected && dates.includes(selected) && (
            <DayDetail
              date={selected}
              day={byDate.get(selected)}
              notes={notes.get(selected) ?? []}
              notStored={answer.not_stored}
              writeFor={writeFor}
              approval={answer.day_approval}
              onChanged={reload}
            />
          )}
        </>
      )}
    </div>
  )
}

/**
 * The period's worked hours against what it asked for.
 *
 * The two figures sit side by side and the bar divides them - the server
 * answers a pair rather than a percentage, because "32h of 40h" and "80%" are
 * not the same sentence and only one of them survives a part-time week
 * (ADR 0017). The share appears once, as the figure that moves against the
 * period before: September and August have different norms, and their shares
 * are what compare (ADR 0023).
 */
function PeriodTotal({ answer, before, unit }: { answer: DaysResponse; before: DaysResponse | null; unit: PeriodUnit }) {
  const { t } = useTranslation()
  // Open days contribute nothing rather than a partial figure: the total says
  // how much work is on the record, and a running day is not on it yet.
  const worked = answer.worked_seconds
  const paused = answer.days.reduce((sum, day) => sum + day.paused_seconds, 0)
  // What has come due, which the hours are measured against: the whole norm
  // of a period that is over, and the part behind today of one still running
  // (ADR 0023). The whole norm is said beside it while they differ.
  const norm = answer.progress.due_seconds
  const whole = answer.progress.norm_seconds
  const over = worked - norm
  const share = shareOfNorm(worked, norm)
  const change = shareChange(share, before ? shareOfNorm(before.worked_seconds, before.progress.due_seconds) : null)

  return (
    <Panel className="space-y-4 p-4 sm:p-5">
      <StatRow>
        <StatTile label={t('myDay.worked')} value={duration(worked)} tone="accent" />
        <StatTile
          label={t('myDay.norm')}
          value={norm > 0 ? duration(norm) : '—'}
          delta={whole > norm ? t(`myDay.wholeNorm.${unit}`, { hours: duration(whole) }) : undefined}
        />
        {share !== null && (
          <StatTile
            label={t('myDay.ofNorm')}
            value={percent(share)}
            delta={change !== null ? `${points(change)} ${t(`period.versus.${unit}`)}` : undefined}
          />
        )}
        <StatTile label={t('myDay.paused')} value={duration(paused)} />
        <StatTile label={t('myDay.daysRecorded')} value={String(answer.days.length)} />
      </StatRow>

      {norm > 0 ? (
        <Progress
          // Clamped at the norm so the bar stays a bar: a period worked over
          // its norm would otherwise fill past the track and say nothing about
          // how far over it went. The figure beside it is not clamped, and
          // that is where the overtime is stated.
          value={Math.min(worked, norm)}
          max={norm}
          label={t(`myDay.progressLabel.${unit}`)}
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
        // A period that owes nothing - a weekend, a full week of leave, a
        // rate of zero - or nothing yet. Said in words rather than drawn as an
        // empty bar, which would read as "nothing done" instead of "nothing due".
        <p className="text-xs text-faint">{whole > 0 ? t('myDay.nothingDueYet') : t(`myDay.noNorm.${unit}`)}</p>
      )}

      {answer.progress.work_rate !== 1 && (
        // Why this person's norm is not the installation's full week. Without
        // it a half-time week reads as a half-hearted one.
        <p className="text-xs text-faint">{t('myDay.partTime', { rate: answer.progress.work_rate })}</p>
      )}
    </Panel>
  )
}

/**
 * One day in the week list: its hours, and the timeline of how it went.
 *
 * A row opens when there is something to open - a worked day's pauses and
 * tasks, a note on it, the form for writing one, or its report. A day with
 * none of those stays a plain line: a button that expands into nothing is a
 * promise the screen does not keep.
 */
function DayRow({
  date,
  day,
  notes,
  today,
  pausesWithheld,
  writable,
  approval,
  selected,
  onSelect,
}: {
  date: string
  day: Day | undefined
  notes: Note[]
  today: boolean
  pausesWithheld: boolean
  writable: boolean
  approval: boolean
  selected: boolean
  onSelect: () => void
}) {
  const { t } = useTranslation()
  const weekday = weekdayName(date)
  const reportable = day !== undefined && (day.report !== null || (!writable && ownAction(day, approval) !== null))
  const openable = day?.kind === 'work' || notes.length > 0 || writable || reportable
  const noted = notes.length > 0 && <NoteLine notes={notes} />
  const reported = day?.report && <ReportMark look={statusLook(day.report.status, approval)} />

  let row: { className: string; content: ReactNode }

  if (!day) {
    row = {
      className: 'flex items-center gap-3 px-4 py-3.5 sm:gap-4 sm:px-5',
      content: (
        <>
          {/* The label and "nothing recorded" are dimmed, the note is not: a
              manager's word on an empty day is not nothing, and greyed out it
              would read as a leftover. */}
          <div className="shrink-0 opacity-55">
            <DayLabel date={date} weekday={weekday} today={today} />
          </div>
          <div className="min-w-0 flex-1">
            <span className="text-sm text-faint opacity-55">{t('myDay.noData')}</span>
            {noted}
          </div>
        </>
      ),
    }
  } else if (day.kind !== 'work') {
    // A day the employee told us they were away. Its own row rather than a bar
    // of nothing: an empty timeline says "worked no hours", and this day was
    // never going to have any (ADR 0017).
    row = {
      className: 'flex items-center gap-3 px-4 py-3.5 sm:gap-4 sm:px-5',
      content: (
        <>
          <DayLabel date={date} weekday={weekday} today={today} />
          <div className="min-w-0 flex-1">
            <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span className="inline-flex items-center gap-1.5 text-sm text-dim">
                <Plane className="size-3.5 shrink-0" />
                {t(`myDay.dayKind.${day.kind}`)}
              </span>
              {reported}
            </span>
            {noted}
          </div>
        </>
      ),
    }
  } else {
    const total = <div className="font-mono text-sm tabular">{duration(day.worked_seconds)}</div>
    const tasks = day.tasks.length > 0 && (
      <div className="text-[11px] text-faint">{t('myDay.taskCount', { count: day.tasks.length })}</div>
    )
    row = {
      className: 'flex flex-col gap-2 px-4 py-3.5 sm:flex-row sm:items-center sm:gap-4 sm:px-5',
      content: (
        <>
          {/* On a phone the day's name and its total share the top line, and
              the bar gets the full width underneath. Keeping the desktop's
              three columns would leave the bar about eighty pixels wide, which
              is not a drawing of a day - it is a smudge. */}
          <div className="flex items-baseline justify-between gap-3 sm:contents">
            <DayLabel date={date} weekday={weekday} today={today} />
            <div className="flex items-baseline gap-2 sm:hidden">
              {tasks}
              {total}
            </div>
          </div>

          <div className="min-w-0 flex-1">
            <Timeline day={day} withheld={pausesWithheld} />
            {/* Wrapping rather than one line: "12:04–21:30" and a break count
                in mono at 320px are a few pixels over, and a clipped end time
                is the half of the pair that says whether the day is finished. */}
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
              {reported}
            </div>
            {noted}
          </div>

          <div className="hidden shrink-0 text-right sm:block">
            {total}
            {tasks && <div className="mt-0.5">{tasks}</div>}
          </div>
        </>
      ),
    }
  }

  if (!openable) return <div className={row.className}>{row.content}</div>

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-expanded={selected}
      className={`${row.className} w-full cursor-pointer text-left transition-colors hover:bg-soft ${selected ? 'bg-soft' : ''}`}
    >
      {row.content}
    </button>
  )
}

/**
 * The newest note on a day, in one line under it. The words themselves rather
 * than an icon and a count: "Your day off is approved" is the whole message,
 * and a badge would make the reader open the day to learn it.
 */
function NoteLine({ notes }: { notes: Note[] }) {
  const { t } = useTranslation()
  const newest = notes.at(-1)
  if (!newest) return null
  return (
    <p className="mt-1.5 flex min-w-0 items-center gap-1.5 text-xs text-dim">
      <MessageSquareText className="size-3 shrink-0 text-accent-2" aria-hidden />
      <span className="sr-only">{t('notes.rowLabel')}: </span>
      <span className="min-w-0 truncate">{newest.text}</span>
      {notes.length > 1 && (
        <span className="shrink-0 font-mono text-[11px] text-faint tabular">{t('notes.more', { count: notes.length - 1 })}</span>
      )}
    </p>
  )
}

/** The colour roles a status names, as literal classes Tailwind can find. */
const TONE_CLASS = {
  good: 'text-good',
  warn: 'text-warn',
  dim: 'text-dim',
} as const

/** The mark each status wears beside its words. */
const STATUS_ICON = {
  approved: CheckCheck,
  waiting: Hourglass,
  reported: Send,
  returned: Undo2,
  changed: RefreshCcw,
} as const

/**
 * Where the day's report stands, in a word and a mark, on the day's own line.
 * The mark is not decoration: this product's gold accent sits 3.7 ΔE from
 * `warn`, and a status told apart by hue alone is told apart by nothing.
 */
function ReportMark({ look }: { look: StatusLook }) {
  const { t } = useTranslation()
  const Icon = STATUS_ICON[look.icon]
  return (
    <span className={`inline-flex items-center gap-1 font-sans text-[11px] ${TONE_CLASS[look.tone]}`}>
      <Icon className="size-3 shrink-0" aria-hidden />
      {t(look.label)}
    </span>
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

/**
 * The opened day: its pauses and its tasks, or why they are not there - and
 * the notes on it, with the form for another where the reader may write one.
 */
function DayDetail({
  date,
  day,
  notes,
  notStored,
  writeFor,
  approval,
  onChanged,
}: {
  date: string
  day: Day | undefined
  notes: Note[]
  notStored: NotStored[]
  writeFor: string | undefined
  approval: boolean
  onChanged: () => void
}) {
  const worked = day?.kind === 'work'
  const noted = notes.length > 0 || writeFor !== undefined
  // A report the day carries, or - on the person's own page - one they may
  // make. On the drill-down there is nothing to show until they report it.
  const reporting = day !== undefined && (day.report !== null || (writeFor === undefined && ownAction(day, approval) !== null))
  // A link can ask for a day with nothing on it - a note since withdrawn.
  // Nothing opens rather than two panels saying "nothing".
  if (!worked && !noted && !reporting) return null

  const panel = noted && <NotesPanel date={date} notes={notes} writeFor={writeFor} onChanged={onChanged} />
  // A note, when there is one, comes first: it is what a person arriving from
  // the notice came to read, and on a phone the pauses above it would be a
  // screen of scrolling. An empty form waits below the record instead.
  const first = notes.length > 0

  return (
    <div className="space-y-4 sm:space-y-5">
      {/* The report above everything: a day sent back, or waiting for the
          reader's answer, is the thing to do on this day - and a person
          arriving from "your day was returned" came to read exactly that. */}
      {reporting && day && <ReportPanel day={day} subject={writeFor} approval={approval} onChanged={onChanged} />}
      {first && panel}
      {worked && <DayRecord day={day} notStored={notStored} />}
      {!first && panel}
    </div>
  )
}

/**
 * The day's report (ADR 0022): where it stands, and what the reader may do.
 *
 * On the person's own page that is reporting the day - once it is finished,
 * where days are approved - or reporting it again once the report no longer
 * stands for it. On the drill-down it is the manager's answer: approve the
 * hours, or send the day back with a reason the person will read.
 */
function ReportPanel({
  day,
  subject,
  approval,
  onChanged,
}: {
  day: Day
  /** Whose day it is, on the drill-down; nothing on the person's own page. */
  subject: string | undefined
  approval: boolean
  onChanged: () => void
}) {
  const { t } = useTranslation()
  const { user } = useSession()
  const report = day.report
  const own = subject === undefined
  const action = own ? ownAction(day, approval) : null
  const answers = !own && report && user ? review(report, approval, user.id) : null

  return (
    <Panel className="space-y-3 p-4 sm:p-5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <h2 className="text-xs font-medium tracking-wide text-dim uppercase">{t('reports.title')}</h2>
        {report && <ReportMark look={statusLook(report.status, approval)} />}
      </div>

      {report ? (
        <ReportFacts day={day} report={report} />
      ) : (
        <p className="text-sm text-faint">{t(own ? 'reports.ownHint' : 'reports.notReported')}</p>
      )}

      {own && report?.status === 'returned' && <p className="text-xs text-faint">{t('reports.returnedHint')}</p>}
      {own && report?.status === 'changed' && <p className="text-xs text-faint">{t('reports.changedHint')}</p>}

      {action && <ReportButton date={day.date} again={action === 'again'} onReported={onChanged} />}
      {answers && report && (answers.approve || answers.sendBack) && (
        <ReviewControls report={report} approve={answers.approve} sendBack={answers.sendBack} onAnswered={onChanged} />
      )}
    </Panel>
  )
}

/** What was reported and when, who answered, and why it came back. */
function ReportFacts({ day, report }: { day: Day; report: Report }) {
  const { t } = useTranslation()
  const moved = movedHours(day, report)
  const reviewer = report.reviewer ?? t('reports.formerReviewer')

  return (
    <div className="space-y-2">
      <p className="font-mono text-xs text-faint tabular">
        {report.kind === 'work'
          ? t('reports.submittedAt', { at: moment(report.submitted_at), hours: duration(report.worked_seconds) })
          : t('reports.submittedAtKind', { at: moment(report.submitted_at), kind: t(`myDay.dayKind.${report.kind}`) })}
      </p>
      {moved && (
        <p className="text-sm text-dim">
          {moved.now === null
            ? t('reports.movedOpen', { reported: duration(moved.reported) })
            : t('reports.moved', { reported: duration(moved.reported), now: duration(moved.now) })}
        </p>
      )}
      {report.reviewed_at && (report.status === 'approved' || report.status === 'returned') && (
        <p className="text-xs text-faint">
          {t(report.status === 'approved' ? 'reports.approvedBy' : 'reports.returnedBy', {
            name: reviewer,
            at: moment(report.reviewed_at),
          })}
        </p>
      )}
      {report.status === 'returned' && report.reason && (
        // The manager's own line breaks, and a long word wrapped rather than
        // pushing the panel wider than a phone - the way a note is drawn.
        <p className="border-l-2 border-warn pl-3 text-sm break-words whitespace-pre-wrap">{report.reason}</p>
      )}
    </div>
  )
}

function ReportButton({ date, again, onReported }: { date: string; again: boolean; onReported: () => void }) {
  const { t } = useTranslation()
  const [pending, setPending] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)

  const send = () => {
    setPending(true)
    setFailed(null)
    api
      .reportDay(date)
      .then(onReported)
      .catch((error: unknown) => {
        // The server's reason as it gave it: "the day is still open; finish
        // it in kasl" says what to do, and "something went wrong" does not.
        setFailed(error instanceof ApiError ? error.message : t('common.error'))
        setPending(false)
      })
  }

  return (
    <div className="space-y-2">
      <Button variant="primary" size="sm" disabled={pending} onClick={send}>
        {t(pending ? 'reports.reporting' : again ? 'reports.again' : 'reports.report')}
      </Button>
      {failed && <p className="text-xs text-bad">{t('reports.failed', { reason: failed })}</p>}
    </div>
  )
}

/**
 * A manager's answer to a report: approve it, or send it back with a reason.
 *
 * Sending back takes a second step, because it needs words - the person has
 * to know what to look at - and the question belongs next to the day it is
 * about, not over the page.
 */
function ReviewControls({
  report,
  approve,
  sendBack,
  onAnswered,
}: {
  report: Report
  approve: boolean
  sendBack: boolean
  onAnswered: () => void
}) {
  const { t } = useTranslation()
  const [writing, setWriting] = useState(false)
  const [reason, setReason] = useState('')
  const [pending, setPending] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  const length = [...reason.trim()].length
  const ready = length > 0 && length <= MAX_REASON_CHARS && !pending

  const fail = (error: unknown) => {
    setFailed(error instanceof ApiError ? error.message : t('common.error'))
    setPending(false)
  }

  const approveIt = () => {
    setPending(true)
    setFailed(null)
    api
      .approveReports([report.id])
      .then((outcome) => {
        // Refused for a reason the server names - the day changed a moment
        // ago, a newer report arrived. Said rather than swallowed, and the
        // week is asked for again either way.
        const refused = outcome.refused[0]
        if (refused) setFailed(refused.error)
        setPending(false)
        onAnswered()
      })
      .catch(fail)
  }

  const sendItBack = () => {
    if (!ready) return
    setPending(true)
    setFailed(null)
    api.returnReport(report.id, reason).then(onAnswered).catch(fail)
  }

  return (
    <div className="space-y-2">
      {!writing && (
        <div className="flex flex-wrap items-center gap-2">
          {approve && (
            <Button variant="primary" size="sm" disabled={pending} onClick={approveIt}>
              {t(pending ? 'reports.approving' : 'reports.approve')}
            </Button>
          )}
          {sendBack && (
            <Button variant="ghost" size="sm" disabled={pending} onClick={() => setWriting(true)}>
              <Undo2 className="size-3.5" aria-hidden />
              {t('reports.sendBack')}
            </Button>
          )}
        </div>
      )}
      {writing && (
        <form
          className="space-y-2"
          onSubmit={(event) => {
            event.preventDefault()
            sendItBack()
          }}
        >
          <label htmlFor={`reason-${report.id}`} className="text-xs text-dim">
            {t('reports.reasonLabel', { date: report.date })}
          </label>
          <Textarea
            id={`reason-${report.id}`}
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            onKeyDown={(event) => {
              // The shortcut every message box has; Enter alone is a new line.
              if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
                event.preventDefault()
                sendItBack()
              }
            }}
            placeholder={t('reports.reasonPlaceholder')}
            autoResize
            maxRows={6}
            rows={2}
            aria-invalid={length > MAX_REASON_CHARS || undefined}
            // 16px on a phone, where iOS zooms into any smaller field.
            className="text-lg sm:text-sm"
            autoFocus
          />
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="min-w-0 flex-1 text-xs text-faint">{t('reports.reasonHint')}</p>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={pending}
                onClick={() => {
                  setWriting(false)
                  setReason('')
                }}
              >
                {t('reports.cancel')}
              </Button>
              <Button type="submit" size="sm" variant="primary" disabled={!ready}>
                {t(pending ? 'reports.sendingBack' : 'reports.sendBack')}
              </Button>
            </div>
          </div>
        </form>
      )}
      {failed && <p className="text-xs text-bad">{t('reports.answerFailed', { reason: failed })}</p>}
    </div>
  )
}

/** A worked day's pauses and tasks, or why they are not there. */
function DayRecord({ day, notStored }: { day: Day; notStored: NotStored[] }) {
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

/**
 * What managers wrote on the day (ADR 0021), and - on the drill-down - the
 * form for another.
 *
 * Not a conversation: no replies, no editing. A note is said once and told to
 * the person on their machine; a correction is a second note, and one on the
 * wrong day or the wrong person is withdrawn.
 */
function NotesPanel({
  date,
  notes,
  writeFor,
  onChanged,
}: {
  date: string
  notes: Note[]
  writeFor: string | undefined
  onChanged: () => void
}) {
  const { t } = useTranslation()
  const { user } = useSession()

  return (
    <Panel className="p-4 sm:p-5">
      <h2 className="text-xs font-medium tracking-wide text-dim uppercase">{t('notes.title')}</h2>
      {notes.length === 0 ? (
        <p className="mt-3 text-sm text-faint">{t('notes.none')}</p>
      ) : (
        <ul className="mt-3 space-y-3.5">
          {notes.map((note) => (
            <NoteItem key={note.id} note={note} withdrawable={user ? mayWithdraw(note, user) : false} onWithdrawn={onChanged} />
          ))}
        </ul>
      )}
      {writeFor !== undefined && <NoteForm userId={writeFor} date={date} onWritten={onChanged} />}
    </Panel>
  )
}

function NoteItem({ note, withdrawable, onWithdrawn }: { note: Note; withdrawable: boolean; onWithdrawn: () => void }) {
  const { t } = useTranslation()
  // Two steps rather than a dialog: withdrawing takes the words out for the
  // person too, and a single stray click should not be able to do that - but
  // the question belongs next to the note it is about, not over the page.
  const [confirming, setConfirming] = useState(false)
  const [pending, setPending] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)

  const withdraw = () => {
    setPending(true)
    setFailed(null)
    api
      .withdrawNote(note.id)
      .then(onWithdrawn)
      .catch((error: unknown) => {
        setFailed(error instanceof ApiError ? error.message : t('common.error'))
        setPending(false)
      })
  }

  return (
    <li className="space-y-1">
      {/* The author's own line breaks, and a long word wrapped rather than
          pushing the panel wider than a phone. */}
      <p className="text-sm break-words whitespace-pre-wrap">{note.text}</p>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-faint">
        <span>
          {note.author ?? t('notes.formerAuthor')} · <span className="font-mono tabular">{moment(note.created_at)}</span>
        </span>
        {withdrawable && !confirming && (
          <button
            type="button"
            onClick={() => setConfirming(true)}
            className="cursor-pointer text-dim underline-offset-2 hover:text-bad hover:underline"
          >
            {t('notes.withdraw')}
          </button>
        )}
      </div>
      {confirming && (
        <div className="flex flex-wrap items-center gap-2 pt-1">
          <span className="text-xs text-dim">{t('notes.withdrawConfirm')}</span>
          <Button size="sm" variant="danger" disabled={pending} onClick={withdraw}>
            {t('notes.withdrawYes')}
          </Button>
          <Button size="sm" variant="ghost" disabled={pending} onClick={() => setConfirming(false)}>
            {t('notes.withdrawNo')}
          </Button>
        </div>
      )}
      {failed && <p className="text-xs text-bad">{t('notes.withdrawFailed', { reason: failed })}</p>}
    </li>
  )
}

/** The longest note the server takes, in characters. Mirrors `notes::MAX_CHARS`. */
const MAX_NOTE_CHARS = 1000

function NoteForm({ userId, date, onWritten }: { userId: string; date: string; onWritten: () => void }) {
  const { t } = useTranslation()
  const [text, setText] = useState('')
  const [pending, setPending] = useState(false)
  const [failed, setFailed] = useState<string | null>(null)
  const length = [...text.trim()].length
  const ready = length > 0 && length <= MAX_NOTE_CHARS && !pending

  const submit = () => {
    if (!ready) return
    setPending(true)
    setFailed(null)
    api
      .addNote(userId, date, text)
      .then(() => {
        setText('')
        setPending(false)
        onWritten()
      })
      .catch((error: unknown) => {
        // The server's reason, as it gave it: "a note is dated at most 366
        // days ahead" says what to change, and "something went wrong" does not.
        setFailed(error instanceof ApiError ? error.message : t('common.error'))
        setPending(false)
      })
  }

  return (
    <form
      className="mt-4 space-y-2 border-t border-line pt-4"
      onSubmit={(event) => {
        event.preventDefault()
        submit()
      }}
    >
      <label htmlFor={`note-${date}`} className="sr-only">
        {t('notes.label', { date })}
      </label>
      <Textarea
        id={`note-${date}`}
        value={text}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          // The shortcut every message box has; Enter alone is a new line,
          // because a note may well have two.
          if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
            event.preventDefault()
            submit()
          }
        }}
        placeholder={t('notes.placeholder')}
        autoResize
        maxRows={6}
        rows={2}
        aria-invalid={length > MAX_NOTE_CHARS || undefined}
        // 16px on a phone: iOS zooms the page into any field smaller than
        // that on focus. `text-lg` is 16px in the line's scale (`text-base`
        // is 14).
        className="text-lg sm:text-sm"
      />
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="min-w-0 flex-1 text-xs text-faint">{t('notes.hint')}</p>
        <div className="flex items-center gap-3">
          {length > MAX_NOTE_CHARS * 0.8 && (
            <span className={`font-mono text-xs tabular ${length > MAX_NOTE_CHARS ? 'text-bad' : 'text-faint'}`}>
              {t('notes.length', { count: length, max: MAX_NOTE_CHARS })}
            </span>
          )}
          <Button type="submit" size="sm" variant="primary" disabled={!ready}>
            {t(pending ? 'notes.adding' : 'notes.add')}
          </Button>
        </div>
      </div>
      {failed && <p className="text-xs text-bad">{t('notes.failed', { reason: failed })}</p>}
    </form>
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
