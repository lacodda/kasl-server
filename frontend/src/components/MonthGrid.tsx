import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import type { TimesheetCell, TimesheetColumn, TimesheetResponse, TimesheetRow } from '@/lib/api'
import { weekdayName } from '@/lib/day'
import { dayOfMonth, isRest } from '@/lib/heatmap'
import { Panel } from '@/components/ui/panel'

/** A column at the end of each row: what it is called, and what it says. */
export interface Total {
  key: string
  label: string
  render: (row: TimesheetRow) => ReactNode
}

/**
 * The frame every view of the month is drawn in: a row per person, a column
 * per date, the totals at the end.
 *
 * One frame for the three views, so a date is in the same place whichever of
 * them is open and switching views moves nothing but the cells. The frame
 * carries the calendar: a column that owes nothing - a weekend nobody moved, a
 * holiday - is shaded, and its header says why on hover.
 *
 * A month is wider than a phone and, with a big team, wider than a laptop. It
 * scrolls inside its own box rather than taking the page with it, with the
 * names frozen at the left; `overscroll-x-contain` stops a swipe that reaches
 * the end of the month from turning into the browser's back gesture.
 */
export function MonthGrid({
  answer,
  caption,
  plan,
  cell,
  totals,
}: {
  answer: TimesheetResponse
  caption: string
  /** A first row saying what each date is worth at a full day. */
  plan?: boolean
  cell: (cell: TimesheetCell, column: TimesheetColumn, row: TimesheetRow) => ReactNode
  totals: Total[]
}) {
  const { t } = useTranslation()

  if (answer.rows.length === 0) {
    return (
      <Panel className="p-4 sm:p-5">
        <p className="text-sm text-dim">{t('team.nobody')}</p>
      </Panel>
    )
  }

  return (
    <Panel className="p-3 sm:p-5">
      <div className="overflow-x-auto overscroll-x-contain">
        <table className="w-full border-separate border-spacing-0 text-sm">
          <caption className="sr-only">{caption}</caption>
          <thead>
            <tr>
              <th scope="col" className="sticky left-0 z-10 bg-raise pr-3 pb-2 text-left align-bottom text-xs font-medium text-dim">
                {t('month.person')}
              </th>
              {answer.dates.map((column) => (
                <DateHeader key={column.date} column={column} />
              ))}
              {totals.map((total) => (
                <th key={total.key} scope="col" className="pb-2 pl-3 text-right align-bottom text-xs font-medium whitespace-nowrap text-dim">
                  {total.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {plan && (
              <tr>
                <th scope="row" className="sticky left-0 z-10 bg-raise py-1 pr-3 text-left text-xs font-normal text-faint">
                  {t('month.plan', { hours: answer.standard_hours })}
                </th>
                {answer.dates.map((column) => (
                  <td key={column.date} className={`py-1 text-center font-mono text-[11px] text-faint tabular ${restClass(column)}`}>
                    {isRest(column) ? '' : column.hours}
                  </td>
                ))}
                <td colSpan={totals.length} />
              </tr>
            )}
            {answer.rows.map((row) => (
              <tr key={row.id}>
                {/* Narrower on a phone: the frozen name column is taken out of
                    the dates' width, and at 320px a 176px column leaves room
                    for about six days of the month. */}
                <th scope="row" className="sticky left-0 z-10 max-w-28 truncate bg-raise py-0.5 pr-2 text-left font-normal sm:max-w-44 sm:pr-3">
                  {/* The name is the whole way into a person from here, and it
                      opens on this month. `py-2` takes it from a 16px strip to
                      a target a thumb can hit. */}
                  <Link
                    to={`/team/${row.id}?${new URLSearchParams({ period: 'month', from: answer.from })}`}
                    className="block truncate py-2 text-sm transition-colors hover:text-accent-2"
                  >
                    {row.display_name}
                  </Link>
                  {row.department && <span className="block truncate text-[11px] text-faint">{row.department}</span>}
                </th>
                {row.days.map((day, at) => {
                  const column = answer.dates[at]
                  if (!column) return null
                  return (
                    <td key={day.date} className={`px-px py-0.5 text-center ${restClass(column)}`}>
                      {cell(day, column, row)}
                    </td>
                  )
                })}
                {totals.map((total) => (
                  <td key={total.key} className="py-0.5 pl-3 text-right font-mono text-xs whitespace-nowrap tabular">
                    {total.render(row)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </Panel>
  )
}

/** The shade of a column that owes nothing at a full day. */
function restClass(column: TimesheetColumn): string {
  return isRest(column) ? 'bg-soft/50' : ''
}

/**
 * A date's header: its number, its weekday, and - where the calendar has a
 * word for it - a mark and the word on hover.
 */
function DateHeader({ column }: { column: TimesheetColumn }) {
  const { t } = useTranslation()
  const special = column.calendar ? t(`month.calendar.${column.calendar}`) : null
  const label = [column.date, special, column.note].filter(Boolean).join(' - ')

  return (
    <th scope="col" title={label} className={`min-w-[30px] px-px pb-2 text-center align-bottom font-normal ${restClass(column)}`}>
      <span className={`block font-mono text-[11px] tabular ${column.calendar ? 'font-semibold text-accent-2' : 'text-dim'}`}>
        {dayOfMonth(column.date)}
      </span>
      <span className="block text-[10px] text-faint">{weekdayName(column.date)}</span>
    </th>
  )
}
