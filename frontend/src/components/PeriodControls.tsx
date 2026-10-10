import { useTranslation } from 'react-i18next'
import { isoDate } from '@/lib/day'
import { elapsed, PERIOD_UNITS, periodOf, shiftPeriod, type Period, type PeriodUnit } from '@/lib/period'
import { Segment, SegmentedControl } from '@/components/ui/segmented-control'
import { PeriodPicker } from '@/components/PeriodPicker'

/**
 * Which period a screen shows: the unit, and back, now, forward within it.
 *
 * The unit is a segmented control rather than a select - three options, all in
 * view, one press to switch and the same press to switch back. Changing the
 * unit keeps the reader where they were: from a week in August, "month" is
 * August, and from this week it is this month.
 */
export function PeriodControls({ period, onChange }: { period: Period; onChange: (next: Period) => void }) {
  const { t } = useTranslation()
  const today = isoDate(new Date())

  const switchUnit = (unit: string) => {
    // Today, when today is in view; otherwise the period's first day - the
    // date the reader was looking at, give or take the unit.
    const anchor = period.from <= today && today <= period.to ? today : period.from
    onChange(periodOf(unit as PeriodUnit, anchor))
  }

  return (
    // A column on a phone, the segment above the arrows: on one 320px row the
    // three words and three buttons leave the arrows no room to be thumbs'
    // targets. A row from `sm`.
    <div className="flex flex-col items-stretch gap-2 sm:flex-row sm:items-center sm:gap-3">
      <SegmentedControl aria-label={t('period.label')} value={period.unit} onValueChange={switchUnit} className="self-center sm:self-auto">
        {PERIOD_UNITS.map((unit) => (
          <Segment key={unit} value={unit}>
            {t(`period.unit.${unit}`)}
          </Segment>
        ))}
      </SegmentedControl>
      <PeriodPicker
        previousLabel={t(`period.previous.${period.unit}`)}
        nextLabel={t(`period.next.${period.unit}`)}
        onPrevious={() => onChange(shiftPeriod(period, -1))}
        onNext={() => onChange(shiftPeriod(period, 1))}
        onNow={() => onChange(periodOf(period.unit, today))}
      >
        {t(`period.now.${period.unit}`)}
      </PeriodPicker>
    </div>
  )
}

/**
 * The period's dates as the header line under a title: one date, or two - and
 * how much of it the figures cover, while it is still running. Everything
 * below is measured to today (ADR 0023), and the line says so rather than
 * leaving a reader to wonder why the month's norm is a third of a month's.
 */
export function PeriodRange({ period }: { period: Period }) {
  const { t } = useTranslation()
  const span = elapsed(period, isoDate(new Date()))
  return (
    <p className="mt-1 font-mono text-xs text-faint tabular">
      {period.from === period.to ? period.from : `${period.from} — ${period.to}`}
      {span === null && <span className="ml-2 font-sans">{t('period.upcoming')}</span>}
      {span !== null && span.to < period.to && <span className="ml-2 font-sans">{t('period.soFar', { date: span.to })}</span>}
    </p>
  )
}
