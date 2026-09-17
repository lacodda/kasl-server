import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import i18n from '@/i18n'
import { api, type TrendResponse } from '@/lib/api'
import { duration } from '@/lib/day'
import { bars, hours, signalPhrase, signalTone, weekLabel } from '@/lib/signals'
import { Panel } from '@/components/ui/panel'
import { BarChart, Baseline, ChartFrame, type BarDatum } from '@/components/ui/bar-chart'

/**
 * One person's twelve weeks, above their week of days.
 *
 * This is where a signal on the dashboard leads, so the reason for the trip is
 * repeated here in words: arriving at a chart and having to re-derive why the
 * server spoke up would make the link a dead end.
 *
 * An empty week keeps its column. Closing the gap up would turn an absence
 * into continuity, and the absence is usually the thing worth seeing.
 */
export function Trend({ userId }: { userId: string }) {
  const { t } = useTranslation()
  // Keyed by the person it answers, so a slow reply for whoever was open
  // before cannot land on the page of the person now being read - the same
  // guard the dashboard puts on its week.
  const [answered, setAnswered] = useState<{ userId: string; trend: TrendResponse | null } | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .userTrend(userId)
      .then((trend) => {
        if (!cancelled) setAnswered({ userId, trend })
      })
      .catch(() => {
        if (!cancelled) setAnswered({ userId, trend: null })
      })
    return () => {
      cancelled = true
    }
  }, [userId])

  // Silence rather than an error: the days below load independently, and a
  // chart that could not be drawn is not worth interrupting the page for.
  const loaded = answered?.userId === userId ? answered.trend : null
  if (!loaded) return null

  const language = i18n.language || 'en'
  const drawn = bars(loaded.weeks)
  const worked = drawn.filter((bar) => bar.worked_seconds !== null)

  const columns: BarDatum[] = drawn.map((bar) => {
    const week = weekLabel(bar.week_start, language)
    return {
      key: bar.week_start,
      value: bar.worked_seconds,
      // No names under the columns on a phone, and the two ends named beneath
      // the chart instead. Measured at 390px: a column is 18px wide and `Jun
      // 22` needs 28, so every label truncated to `Ju…` - twelve of them, an
      // axis that takes a real one's height and says nothing. Thinning them
      // out does not help, because the width is the column's and not the
      // row's. The shape is what this chart is for, the span is named under
      // it, and every column still says its own week and hours on touch.
      label: <span className="hidden sm:inline">{week}</span>,
      title:
        bar.worked_seconds === null
          ? t('trend.emptyWeek', { week })
          : t('trend.weekWorked', { week, hours: duration(bar.worked_seconds) }),
    }
  })

  // The scale the bars and the median line are both drawn against. Stated once
  // rather than left to the chart, because a baseline resolved against a
  // different ceiling than the columns would sit at the wrong height - and
  // look exactly as convincing as one at the right one.
  const ceiling = Math.max(...worked.map((bar) => bar.worked_seconds ?? 0), loaded.median_seconds ?? 0, 1)

  return (
    <Panel className="space-y-4 p-4 sm:p-5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1">
        <h2 className="text-sm font-medium">{t('trend.title', { count: loaded.weeks.length })}</h2>
        {loaded.median_seconds !== null && (
          // Named here as well as drawn on the chart: every signal about this
          // person is measured against it, and the figure is worth having in
          // words for a reader who is not reading heights.
          <p className="font-mono text-xs text-faint tabular">{t('trend.median', { hours: hours(loaded.median_seconds) })}</p>
        )}
      </div>

      {worked.length === 0 ? (
        <p className="text-sm text-faint">{t('trend.nothing')}</p>
      ) : (
        /* `Baseline` places itself as a percentage of the frame, measured from
         * the frame's bottom - but the frame also holds the row of week labels
         * `BarChart` draws under the plot, so that percentage is of the wrong
         * height and the line lands above where it belongs. Measured here at
         * 390px: the frame is 134px, the plot 112px, and the median sat 22px
         * high, which on this chart is about four hours. A chart never
         * announces an error like that; it just quietly reads wrong.
         *
         * `BarChart` offers no way to leave the labels out, and `bottom` is a
         * percentage of the padding box, so neither padding nor a wrapper
         * moves the zero. Pushing the line back down by a measured constant
         * was tried and abandoned: the offset is the label row on a desktop
         * and the gap that survives it on a phone, two numbers that would go
         * stale the first time dowel changed either.
         *
         * So the line is drawn here, over a box that is exactly the plot: one
         * `--plot` tall, on top of the chart, which puts its 0% on the axis by
         * construction rather than by correction. Same ceiling as the bars, so
         * the two cannot disagree.
         *
         * Ordered to a wish on dowel, because every consumer of `Baseline`
         * with `BarChart` meets this, and it is invisible when it is wrong -
         * the line lands at a height that looks like an answer. */
        <ChartFrame gutter={loaded.median_seconds !== null} className="[--plot:72px] sm:[--plot:112px]">
          {/* `--plot` is written on the chart as well as on the frame, and
              with the same values: `size` always sets it on the chart itself,
              where the overlay above cannot read it. Stating it twice is what
              keeps the bars and the line on one scale - and the pair is here,
              on two adjacent lines, rather than split across two files. */}
          <BarChart
            bars={columns}
            max={ceiling}
            className="[--plot:72px] sm:[--plot:112px]"
            label={t('trend.chartLabel', { count: loaded.weeks.length })}
          />
          {loaded.median_seconds !== null && (
            <div className="pointer-events-none absolute inset-x-0 top-0 h-[var(--plot)]">
              {/* Half a label down. `bottom` puts the *bottom* of a row the
                  height of the label at the value, and the rule is centred
                  inside that row - so it is drawn half a label above the
                  number it names. Measured on this chart: 5.5px high, about an
                  hour of a working week, and it went to 10.5 with the sign the
                  other way round, which is how the direction was settled. */}
              <Baseline value={loaded.median_seconds} max={ceiling} className="translate-y-1/2">
                {t('trend.medianShort', { hours: hours(loaded.median_seconds) })}
              </Baseline>
            </div>
          )}
        </ChartFrame>
      )}

      {/* The span the columns cover, named once. This is the phone's axis -
          the per-column labels do not fit there and are left out above - and
          it goes away at `sm`, where each column names its own week and this
          line would only repeat two of them. */}
      {worked.length > 0 && (
        <p className="font-mono text-[10px] text-faint tabular sm:hidden">
          {weekLabel(drawn[0]!.week_start, language)} — {weekLabel(drawn.at(-1)!.week_start, language)}
        </p>
      )}

      {loaded.signals.length > 0 && (
        <div className="space-y-1 border-t border-line pt-3">
          {loaded.signals.map((signal, index) => {
            const phrase = signalPhrase(signal)
            return (
              <p key={`${signal.kind}-${index}`} className={`text-xs ${signalTone(signal.kind) === 'warn' ? 'text-warn' : 'text-info'}`}>
                {t(phrase.key, phrase.values)}
              </p>
            )
          })}
        </div>
      )}
    </Panel>
  )
}
