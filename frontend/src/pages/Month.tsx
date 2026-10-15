import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useSearchParams } from 'react-router'
import { Download } from 'lucide-react'
import { api, downloadTimesheet, type TimesheetCell, type TimesheetResponse, type TimesheetRow } from '@/lib/api'
import { duration, monthName } from '@/lib/day'
import { busiest, currentMonth, isMonth, shiftMonth, squares, step, type Square } from '@/lib/heatmap'
import { daysAway, GLYPH, hoursShort, measuredAgainst, standingOf, VIEWS, viewOf, type View } from '@/lib/timesheet'
import { Button } from '@/components/ui/button'
import { Segment, SegmentedControl } from '@/components/ui/segmented-control'
import { PeriodPicker } from '@/components/PeriodPicker'
import { MonthGrid, type Total } from '@/components/MonthGrid'

/**
 * The team's month, three ways (ADR 0024).
 *
 * One answer - the month's timesheet - and three views over it: the
 * **timesheet** (how much, against the norm, and what a company files), the
 * **reports** (whether each day was put a name to, and in time) and the
 * **shape** (the pattern a heatmap shows). One request rather than one per
 * view, so the three cannot disagree about a month, and switching between
 * them is instant.
 *
 * The month and the view are in the URL: a month can be linked, and the back
 * button from a person's page returns to the view that was open.
 */
export function Month() {
  const { t } = useTranslation()
  const [params, setParams] = useSearchParams()
  const asked = params.get('month')
  const month = isMonth(asked) ? asked : currentMonth()
  const view = viewOf(params.get('view'))

  // Replacing rather than pushing: paging through a year must not leave
  // twelve entries for the back button to wade through.
  const go = useCallback(
    (next: { month?: string; view?: View }) =>
      setParams({ month: next.month ?? month, view: next.view ?? view }, { replace: true }),
    [setParams, month, view],
  )

  // Keyed by the month it answers, so a slow reply for the month just left
  // cannot land as if it belonged to this one.
  const [loaded, setLoaded] = useState<{ month: string; answer: TimesheetResponse | null } | null>(null)
  useEffect(() => {
    let cancelled = false
    api
      .teamTimesheet(month)
      .then((answer) => {
        if (!cancelled) setLoaded({ month, answer })
      })
      .catch(() => {
        if (!cancelled) setLoaded({ month, answer: null })
      })
    return () => {
      cancelled = true
    }
  }, [month])

  const current = loaded?.month === month ? loaded : null
  const answer = current?.answer ?? null
  const failed = current !== null && current.answer === null

  return (
    <div className="mx-auto max-w-[96rem] space-y-5">
      <header className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between lg:gap-4">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold">{t('month.title')}</h1>
          <p className="mt-1 text-xs text-faint">{monthName(month)}</p>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
          <SegmentedControl
            aria-label={t('month.view.label')}
            value={view}
            onValueChange={(next) => go({ view: viewOf(next) })}
            className="self-center sm:self-auto"
          >
            {VIEWS.map((option) => (
              <Segment key={option} value={option}>
                {t(`month.view.${option}`)}
              </Segment>
            ))}
          </SegmentedControl>
          <PeriodPicker
            previousLabel={t('month.previous')}
            nextLabel={t('month.next')}
            onPrevious={() => go({ month: shiftMonth(month, -1) })}
            onNext={() => go({ month: shiftMonth(month, 1) })}
            onNow={() => go({ month: currentMonth() })}
          >
            {t('month.now')}
          </PeriodPicker>
          <TimesheetDownload month={month} />
        </div>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {answer && view === 'timesheet' && <Timesheet answer={answer} />}
      {answer && view === 'reports' && <Reports answer={answer} />}
      {answer && view === 'shape' && <Shape answer={answer} />}
    </div>
  )
}

/**
 * The month as the workbook a company files: both grids, the timesheet and
 * the reports, whichever view is open - the file is the record, and the
 * record has both.
 */
function TimesheetDownload({ month }: { month: string }) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const [failed, setFailed] = useState(false)

  const download = () => {
    setBusy(true)
    setFailed(false)
    downloadTimesheet('team', month)
      .catch(() => setFailed(true))
      .finally(() => setBusy(false))
  }

  return (
    <div className="flex items-center gap-2">
      {failed && (
        <span role="alert" className="text-xs text-bad">
          {t('export.failed')}
        </span>
      )}
      {/* The phone's height for a finger, handed back at `sm`, as the
          month's arrows beside it do. */}
      <Button size="sm" className="h-11 w-full sm:h-7 sm:w-auto" disabled={busy} onClick={download}>
        <Download aria-hidden className="size-3.5" />
        {busy ? t('export.preparing') : t('month.download')}
      </Button>
    </div>
  )
}

// The timesheet ----------------------------------------------------------------

/**
 * How much: the hours of each day worked, a letter for each day away, and
 * what was owed and never recorded - against what the month has asked so far.
 */
function Timesheet({ answer }: { answer: TimesheetResponse }) {
  const { t } = useTranslation()

  const totals: Total[] = [
    { key: 'worked', label: t('month.total.worked'), render: (row) => hoursShort(row.worked_seconds) },
    { key: 'due', label: t('month.total.due'), render: (row) => <span className="text-dim">{hoursShort(measuredAgainst(row))}</span> },
    { key: 'away', label: t('month.total.away'), render: (row) => count(daysAway(row)) },
    {
      key: 'unrecorded',
      label: t('month.total.unrecorded'),
      render: (row) => (row.counts.unrecorded > 0 ? <span className="text-warn">{row.counts.unrecorded}</span> : count(0)),
    },
  ]

  return (
    <>
      <MonthGrid
        answer={answer}
        caption={t('month.caption.timesheet', { month: monthName(answer.month) })}
        plan
        cell={(cell) => <Entry cell={cell} />}
        totals={totals}
      />
      <Legend
        items={[
          { mark: <Swatch className="bg-soft text-dim">{t('month.code.vacation')}</Swatch>, label: t('month.legend.vacation') },
          { mark: <Swatch className="bg-soft text-dim">{t('month.code.sick')}</Swatch>, label: t('month.legend.sick') },
          { mark: <Swatch className="bg-soft text-dim">{t('month.code.day_off')}</Swatch>, label: t('month.legend.dayOff') },
          { mark: <Swatch className="bg-warn-soft text-warn">?</Swatch>, label: t('month.legend.unrecorded') },
          { mark: <Swatch className="border border-dashed border-accent/60">…</Swatch>, label: t('month.legend.open') },
          { mark: <Swatch className="text-faint">8</Swatch>, label: t('month.legend.ahead') },
          { mark: <Swatch className="bg-soft/50" />, label: t('month.legend.rest') },
        ]}
        note={t('month.legend.due')}
      />
    </>
  )
}

/** A zero in a count column: present, but quiet. */
function count(value: number) {
  return value > 0 ? value : <span className="text-faint">0</span>
}

/**
 * One date of the timesheet.
 *
 * The label carries the whole sentence - the exact hours, the kind of leave,
 * the holiday's name - because a grid of figures and letters says nothing to
 * a screen reader, and a tenth of an hour is not the exact figure.
 */
function Entry({ cell }: { cell: TimesheetCell }) {
  const { t } = useTranslation()
  const box = 'mx-auto flex h-7 min-w-[26px] items-center justify-center rounded-[5px] px-0.5 font-mono text-xs tabular'

  switch (cell.state) {
    case 'worked': {
      const label = t('month.cell.worked', { date: cell.date, hours: duration(cell.worked_seconds) })
      return (
        <span role="img" aria-label={label} title={label} className={box}>
          {hoursShort(cell.worked_seconds ?? 0)}
        </span>
      )
    }
    case 'open': {
      const label = t('month.cell.open', { date: cell.date })
      return (
        // A running day is outlined rather than given a figure: it has no
        // total yet, and its hours so far would read as a short day.
        <span role="img" aria-label={label} title={label} className={`${box} border border-dashed border-accent/60 text-faint`}>
          …
        </span>
      )
    }
    case 'away': {
      const kind = cell.kind ?? 'day_off'
      const label = t('month.cell.away', { date: cell.date, kind: t(`month.kind.${kind}`) })
      return (
        <span role="img" aria-label={label} title={label} className={`${box} bg-soft text-dim`}>
          {t(`month.code.${kind}`)}
        </span>
      )
    }
    case 'unrecorded': {
      const label = t('month.cell.unrecorded', { date: cell.date })
      return (
        <span role="img" aria-label={label} title={label} className={`${box} bg-warn-soft text-warn`}>
          ?
        </span>
      )
    }
    case 'ahead': {
      const label = t('month.cell.ahead', { date: cell.date, hours: duration(cell.norm_seconds) })
      return (
        <span role="img" aria-label={label} title={label} className={`${box} text-faint`}>
          {hoursShort(cell.norm_seconds)}
        </span>
      )
    }
    case 'free':
      // Nothing to say: the column's header already says it is a weekend or
      // a holiday. Not even a visually hidden word - an absolutely positioned
      // label in a cell far to the right widens the page on a phone, outside
      // the box the month scrolls in.
      return null
  }
}

// The reports -------------------------------------------------------------------

/**
 * Whether each day was put a name to, and in time: before the person's next
 * working day began (ADR 0024). The mark says when; the fill says what a
 * manager made of it.
 */
function Reports({ answer }: { answer: TimesheetResponse }) {
  const { t } = useTranslation()

  const totals: Total[] = [
    { key: 'on_time', label: t('month.total.onTime'), render: (row) => count(row.counts.on_time) },
    {
      key: 'late',
      label: t('month.total.late'),
      render: (row) => (row.counts.late > 0 ? <span className="text-warn">{row.counts.late}</span> : count(0)),
    },
    {
      key: 'missing',
      label: t('month.total.missing'),
      render: (row) => (row.counts.missing > 0 ? <span className="text-bad">{row.counts.missing}</span> : count(0)),
    },
    ...(answer.day_approval ? [{ key: 'approved', label: t('month.total.approved'), render: (row: TimesheetRow) => count(row.counts.approved) }] : []),
  ]

  return (
    <>
      <MonthGrid
        answer={answer}
        caption={t('month.caption.reports', { month: monthName(answer.month) })}
        cell={(cell) => <ReportMark cell={cell} approval={answer.day_approval} />}
        totals={totals}
      />
      <Legend
        items={[
          { mark: <Swatch className="text-good">{GLYPH.on_time}</Swatch>, label: t('month.legend.onTime') },
          { mark: <Swatch className="text-warn">{GLYPH.late}</Swatch>, label: t('month.legend.late') },
          { mark: <Swatch className="text-bad">{GLYPH.missing}</Swatch>, label: t('month.legend.missing') },
          { mark: <Swatch className="text-faint">{GLYPH.pending}</Swatch>, label: t('month.legend.pending') },
          ...(answer.day_approval ? [{ mark: <Swatch className="bg-good-soft text-good">{GLYPH.on_time}</Swatch>, label: t('month.legend.approved') }] : []),
          { mark: <Swatch className="bg-warn-soft" />, label: t('month.legend.attention') },
        ]}
        note={t(answer.day_approval ? 'month.legend.deadline' : 'month.legend.deadlineNoApproval')}
      />
    </>
  )
}

const MARK_TONE = {
  on_time: 'text-good',
  late: 'text-warn',
  missing: 'text-bad',
  pending: 'text-faint',
} as const

function ReportMark({ cell, approval }: { cell: TimesheetCell; approval: boolean }) {
  const { t } = useTranslation()
  if (cell.filed === null) return null

  const standing = standingOf(cell.report)
  const fill = standing === 'approved' ? 'bg-good-soft' : standing === 'attention' ? 'bg-warn-soft' : ''
  // A report nobody has answered is waiting where days are approved, and
  // simply reported where they are not - the words the personal page uses.
  const status = cell.report === 'submitted' ? (approval ? 'waiting' : 'reported') : cell.report
  const label = [t(`month.filed.${cell.filed}`, { date: cell.date }), status ? t(`reports.status.${status}`) : null].filter(Boolean).join(' - ')

  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={`mx-auto flex h-7 w-[26px] items-center justify-center rounded-[5px] text-sm font-semibold ${MARK_TONE[cell.filed]} ${fill}`}
    >
      {GLYPH[cell.filed]}
    </span>
  )
}

// The shape -----------------------------------------------------------------------

/** The five shades, as literal class names Tailwind can find in the source. */
const FILL = ['fill-1', 'fill-2', 'fill-3', 'fill-4', 'fill-5'] as const

/**
 * The pattern: a square per day, shaded against the busiest day anywhere in
 * the month (ADR 0015). An empty square is the one a manager is really
 * scanning for, so it is drawn as absence and never as a zero.
 */
function Shape({ answer }: { answer: TimesheetResponse }) {
  const { t } = useTranslation()
  const ceiling = useMemo(() => busiest(answer.rows), [answer])
  // Each row's squares once, looked up by date as the grid draws them.
  const grid = useMemo(
    () => new Map(answer.rows.map((row) => [row.id, new Map(squares(row, answer.dates, ceiling).map((square) => [square.date, square]))])),
    [answer, ceiling],
  )
  const totals: Total[] = [
    {
      key: 'worked',
      label: t('month.total.worked'),
      // A row with nothing in it says so in words: a bare `0h 0m` is a claim
      // about how much someone worked, and nobody made it.
      render: (row) =>
        row.days.some((day) => day.kind !== null) ? duration(row.worked_seconds) : <span className="text-faint">{t('month.noData')}</span>,
    },
  ]

  return (
    <>
      <MonthGrid
        answer={answer}
        caption={t('month.caption.shape', { month: monthName(answer.month) })}
        cell={(cell, _column, row) => {
          const square = grid.get(row.id)?.get(cell.date)
          return square ? <ShapeSquare square={square} /> : null
        }}
        totals={totals}
      />
      <div className="flex flex-wrap items-center gap-x-6 gap-y-2 px-1 text-[11px] text-faint">
        <div className="flex items-center gap-1.5">
          <span>{t('month.shape.less')}</span>
          {FILL.map((fill) => (
            <span key={fill} className={`size-3 rounded-[3px] ${fill}`} />
          ))}
          <span>{t('month.shape.more')}</span>
          {/* The scale is relative, so its top step is named: without this
              the last square looks like an absolute standard for a full day. */}
          {ceiling !== null && <span className="ml-1">· {t('month.shape.busiest', { hours: duration(ceiling) })}</span>}
        </div>
        <div className="flex flex-wrap items-center gap-4">
          <span className="flex items-center gap-1.5">
            <span className="size-3 rounded-[3px] bg-soft" />
            {t('month.shape.none')}
          </span>
          <span className="flex items-center gap-1.5">
            <span className="size-3 rounded-[3px] border border-dashed border-accent/60" />
            {t('month.legend.open')}
          </span>
          <span className="flex items-center gap-1.5">
            <span className="size-3 rounded-[3px] border border-line bg-raise" />
            {t('month.shape.away')}
          </span>
        </div>
      </div>
    </>
  )
}

/**
 * One square. Nothing recorded is drawn as the bare surface - no fill at all -
 * so an empty month reads as empty rather than as a wall of the palest shade.
 * A day away is outlined, not shaded: it was recorded, and it was not worked.
 */
function ShapeSquare({ square }: { square: Square }) {
  const { t } = useTranslation()

  const label =
    square.kind === 'worked'
      ? t('month.cell.worked', { date: square.date, hours: duration(square.seconds) })
      : square.kind === 'open'
        ? t('month.cell.open', { date: square.date })
        : square.kind === 'away'
          ? t('month.cell.away', { date: square.date, kind: t(`month.kind.${square.away ?? 'day_off'}`) })
          : t(square.rest ? 'month.cell.free' : 'month.cell.none', { date: square.date })

  const fill =
    square.kind === 'worked' && square.intensity !== null
      ? FILL[step(square.intensity) - 1]
      : square.kind === 'open'
        ? 'border border-dashed border-accent/60'
        : square.kind === 'away'
          ? 'border border-line bg-raise'
          : square.rest
            ? 'bg-softer'
            : 'bg-soft'

  return <div role="img" aria-label={label} title={label} className={`mx-auto size-4 rounded-[3px] ${fill}`} />
}

// Legends --------------------------------------------------------------------------

/** A legend's sample: the same box the grid draws, at the legend's size. */
function Swatch({ className = '', children }: { className?: string; children?: React.ReactNode }) {
  return (
    <span className={`flex size-5 items-center justify-center rounded-[4px] font-mono text-[10px] font-semibold ${className}`}>{children}</span>
  )
}

function Legend({ items, note }: { items: { mark: React.ReactNode; label: string }[]; note?: string }) {
  return (
    <div className="space-y-2 px-1 text-[11px] text-faint">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        {items.map(({ mark, label }) => (
          <span key={label} className="flex items-center gap-1.5">
            {mark}
            {label}
          </span>
        ))}
      </div>
      {note && <p>{note}</p>}
    </div>
  )
}
