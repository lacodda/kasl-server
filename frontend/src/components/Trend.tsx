import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import i18n from '@/i18n'
import { api, type TrendResponse } from '@/lib/api'
import { duration } from '@/lib/day'
import { bars, hours, signalPhrase, signalTone, weekLabel } from '@/lib/signals'
import { Panel } from '@/components/ui/panel'

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

  const drawn = bars(loaded.weeks)
  const worked = drawn.filter((bar) => !bar.empty)

  return (
    <Panel className="space-y-4 p-5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-1">
        <h2 className="text-sm font-medium">{t('trend.title', { count: loaded.weeks.length })}</h2>
        {loaded.median_seconds !== null && (
          // Named, because every signal about this person is measured against
          // it. A chart without its baseline invites the reader to invent one.
          <p className="font-mono text-xs text-faint tabular">{t('trend.median', { hours: hours(loaded.median_seconds) })}</p>
        )}
      </div>

      {worked.length === 0 ? (
        <p className="text-sm text-faint">{t('trend.nothing')}</p>
      ) : (
        <div className="flex items-end gap-1 border-b border-line pt-2">
          {drawn.map((bar) => {
            const label = bar.empty
              ? t('trend.emptyWeek', { week: weekLabel(bar.week_start, i18n.language || 'en') })
              : t('trend.weekWorked', {
                  week: weekLabel(bar.week_start, i18n.language || 'en'),
                  hours: duration(bar.worked_seconds),
                })
            return (
              <div key={bar.week_start} className="flex min-w-0 flex-1 flex-col items-center gap-1.5">
                {/* The track carries the height itself, in pixels. A
                    percentage height resolves against the parent's own
                    height, and a flex item sized from its content gives the
                    child no base to be a percentage of - so every bar
                    computed to zero and the chart rendered as bare axis
                    labels. Found by looking at it: the dotted empty-week
                    rules have a fixed `h-px` and were all that showed. */}
                <div className="flex h-28 w-full items-end" role="img" aria-label={label} title={label}>
                  {bar.empty ? (
                    // A gap, drawn as one: a dotted floor rather than a bar of
                    // no height, which would be indistinguishable from a week
                    // that has not rendered.
                    <div className="mx-auto h-1 w-3/5 rounded-t-[3px] border-x border-t border-dashed border-line-2" />
                  ) : (
                    <div
                      className="mx-auto w-3/5 rounded-t-[3px] bg-accent"
                      // Percentage of the tallest week, with a floor so a very
                      // short week is still a bar rather than nothing.
                      style={{ height: `${Math.max(bar.height * 90, 3)}%` }}
                    />
                  )}
                </div>
                <span className="w-full truncate text-center text-[10px] text-faint tabular">
                  {weekLabel(bar.week_start, i18n.language || 'en')}
                </span>
              </div>
            )
          })}
        </div>
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
