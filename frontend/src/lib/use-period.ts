import { useCallback } from 'react'
import { useSearchParams } from 'react-router'
import { isoDate } from '@/lib/day'
import { periodFromParams, periodParams, type Period, type PeriodUnit } from '@/lib/period'

/**
 * The period a screen shows, kept in its URL.
 *
 * In the URL rather than in state, so a period can be linked, survives a visit
 * to a person's page and the browser's back button, and a reload shows what
 * was on the screen. Setting it replaces every parameter: `date` - the day a
 * notice linked to - is read on arrival and belongs to the period that was
 * open then.
 *
 * A new object on every render: callers depend on its `from` and `to`, which
 * are strings and compare by value.
 */
export function usePeriod(fallbackUnit: PeriodUnit = 'week'): [Period, (next: Period) => void] {
  const [params, setParams] = useSearchParams()
  const period = periodFromParams(params, isoDate(new Date()), fallbackUnit)
  // Replacing rather than pushing: paging through twelve weeks must not leave
  // twelve entries for the back button to wade through before it leaves the
  // screen. The entry still holds the period, so back from a person's page
  // returns to it.
  const setPeriod = useCallback((next: Period) => setParams(periodParams(next), { replace: true }), [setParams])
  return [period, setPeriod]
}
