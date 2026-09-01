import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { api, type HeatmapResponse, type HeatmapRow } from '@/lib/api'
import { duration } from '@/lib/day'
import { currentMonth, dayOfMonth, monthDates, shiftMonth, squares, step, type Square } from '@/lib/heatmap'
import { Panel } from '@/components/ui/Panel'
import { Button } from '@/components/ui/Button'

/**
 * The team's month as a grid: a row per person, a square per day.
 *
 * Its own screen rather than a band under the dashboard. The dashboard is
 * about now and this week and pages by week; this is about the shape of a
 * month and pages by month, and two period pickers on one page teach a reader
 * to distrust both.
 *
 * The square is the whole design, and it has four states rather than a number:
 * nothing recorded, a day still running, a finished day, and a weekend - which
 * is a fact about the calendar, not a claim that anybody was off. An empty
 * square is the one a manager is really scanning for, so it is drawn as
 * absence and never as a zero (ADR 0015).
 */
export function Heatmap() {
  const { t } = useTranslation()
  const [month, setMonth] = useState(currentMonth)
  // Keyed by the month it answers, so a slow reply for the month just left
  // cannot land as if it belonged to this one.
  const [loaded, setLoaded] = useState<{ month: string; answer: HeatmapResponse | null } | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .teamHeatmap(month)
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

  const goto = useCallback((by: number) => setMonth((from) => shiftMonth(from, by)), [])

  return (
    <div className="mx-auto max-w-6xl space-y-5">
      <header className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold">{t('heatmap.title')}</h1>
          <p className="mt-1 font-mono text-xs text-faint tabular">{month}</p>
        </div>
        <div className="flex items-center gap-1.5">
          <Button variant="icon" size="iconMd" aria-label={t('heatmap.previousMonth')} onClick={() => goto(-1)}>
            <ChevronLeft />
          </Button>
          <Button size="sm" onClick={() => setMonth(currentMonth())}>
            {t('heatmap.thisMonth')}
          </Button>
          <Button variant="icon" size="iconMd" aria-label={t('heatmap.nextMonth')} onClick={() => goto(1)}>
            <ChevronRight />
          </Button>
        </div>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {answer && <Grid answer={answer} />}
    </div>
  )
}

function Grid({ answer }: { answer: HeatmapResponse }) {
  const { t } = useTranslation()
  const dates = useMemo(() => monthDates(answer.from, answer.to), [answer.from, answer.to])

  if (answer.rows.length === 0) {
    return (
      <Panel className="p-5">
        <p className="text-sm text-dim">{t('team.nobody')}</p>
      </Panel>
    )
  }

  return (
    <>
      <Panel className="p-5">
        {/* A month of squares is wider than a phone and, with a big team,
            wider than a laptop. It scrolls inside its own box rather than
            taking the page with it. */}
        <div className="overflow-x-auto">
          <table className="w-full border-separate border-spacing-0 text-sm">
            <caption className="sr-only">{t('heatmap.caption', { month: answer.month })}</caption>
            <thead>
              <tr>
                <th scope="col" className="sticky left-0 z-10 bg-raise pb-2 pr-3 text-left text-xs font-medium text-dim">
                  {t('heatmap.person')}
                </th>
                {dates.map((date) => (
                  <th key={date} scope="col" className="pb-2 text-center font-mono text-[10px] font-normal text-faint tabular">
                    {dayOfMonth(date)}
                  </th>
                ))}
                <th scope="col" className="pb-2 pl-3 text-right text-xs font-medium text-dim">
                  {t('heatmap.total')}
                </th>
              </tr>
            </thead>
            <tbody>
              {answer.rows.map((row) => (
                <PersonRow key={row.user_id} row={row} dates={dates} busiest={answer.busiest_seconds} />
              ))}
            </tbody>
          </table>
        </div>
      </Panel>
      <Legend busiest={answer.busiest_seconds} />
    </>
  )
}

function PersonRow({ row, dates, busiest }: { row: HeatmapRow; dates: string[]; busiest: number | null }) {
  const { t } = useTranslation()
  const grid = useMemo(() => squares(row, dates, busiest), [row, dates, busiest])
  const nothing = row.days.length === 0

  return (
    <tr className="group">
      <th scope="row" className="sticky left-0 z-10 max-w-44 truncate bg-raise py-1 pr-3 text-left font-normal">
        <Link to={`/team/${row.user_id}`} className="block truncate text-sm transition-colors hover:text-accent-2">
          {row.display_name}
        </Link>
        {row.department && <span className="block truncate text-[11px] text-faint">{row.department}</span>}
      </th>
      {grid.map((square) => (
        <td key={square.date} className="px-px py-1">
          <Cell square={square} />
        </td>
      ))}
      <td className="py-1 pl-3 text-right font-mono text-xs tabular">
        {/* A row with nothing in it says so in words. A bare `0h 0m` is a
            claim about how much someone worked, and nobody made it. */}
        {nothing ? <span className="text-faint">{t('heatmap.noData')}</span> : duration(row.worked_seconds)}
      </td>
    </tr>
  )
}

/** The five shades, as literal class names Tailwind can find in the source. */
const FILL = ['fill-1', 'fill-2', 'fill-3', 'fill-4', 'fill-5'] as const

/**
 * One square.
 *
 * Nothing recorded is drawn as the bare surface - no fill at all - so an empty
 * month reads as empty rather than as a wall of the palest shade. A weekend
 * with no data is dimmer still, which is the only thing the calendar earns:
 * it says "this is a Saturday", never "they were off".
 *
 * The title is the accessible label too: a colour-only grid says nothing to a
 * screen reader, and nothing to anyone who cannot separate five golds.
 */
function Cell({ square }: { square: Square }) {
  const { t } = useTranslation()

  const label =
    square.kind === 'worked'
      ? t('heatmap.cell.worked', { date: square.date, hours: duration(square.seconds) })
      : square.kind === 'open'
        ? t('heatmap.cell.open', { date: square.date })
        : t(square.weekend ? 'heatmap.cell.weekend' : 'heatmap.cell.none', { date: square.date })

  const fill =
    square.kind === 'worked' && square.intensity !== null
      ? FILL[step(square.intensity) - 1]
      : square.kind === 'open'
        ? // A running day is outlined rather than filled: it has no figure to
          // shade, and any fill would be a number the server did not give.
          'border border-dashed border-accent/60'
        : square.weekend
          ? 'bg-softer'
          : 'bg-soft'

  return (
    <div role="img" aria-label={label} title={label} className={`mx-auto size-4 rounded-[3px] ${fill}`} />
  )
}

/** What the shades mean, in the same classes the grid paints with. */
function Legend({ busiest }: { busiest: number | null }) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-wrap items-center gap-x-6 gap-y-2 px-1 text-[11px] text-faint">
      <div className="flex items-center gap-1.5">
        <span>{t('heatmap.legend.less')}</span>
        {FILL.map((fill) => (
          <span key={fill} className={`size-3 rounded-[3px] ${fill}`} />
        ))}
        <span>{t('heatmap.legend.more')}</span>
        {/* The scale is relative, so it is named. Without this the darkest
            square looks like an absolute standard for a full day - which this
            server has no opinion about until norms arrive. */}
        {busiest !== null && <span className="ml-1">· {t('heatmap.legend.busiest', { hours: duration(busiest) })}</span>}
      </div>
      <div className="flex items-center gap-4">
        <span className="flex items-center gap-1.5">
          <span className="size-3 rounded-[3px] bg-soft" />
          {t('heatmap.legend.none')}
        </span>
        <span className="flex items-center gap-1.5">
          <span className="size-3 rounded-[3px] border border-dashed border-accent/60" />
          {t('heatmap.legend.open')}
        </span>
        <span className="flex items-center gap-1.5">
          <span className="size-3 rounded-[3px] bg-softer" />
          {t('heatmap.legend.weekend')}
        </span>
      </div>
    </div>
  )
}
