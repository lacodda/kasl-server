import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { cn, useLocale } from 'dowel-ui'
import {
  addMonths,
  firstDayOfWeek,
  keyStep,
  monthGrid,
  parts,
  today,
  weekdayNames,
  type IsoDate,
} from './calendar-math'

/*
 * Calendar - a month of days.
 *
 * The sums live next door in `calendar-math`, which has no React in it; this
 * is the grid that draws them and the keyboard that moves around it.
 *
 * One tab stop for the whole grid, arrows within - the arrangement a radio
 * group has, and the reason a calendar is usable at all: forty-two tab stops
 * is not a control. Arrows move a cursor and only Enter chooses, so a product
 * listening for a change does not receive five dates on the way to the sixth.
 *
 * The cursor is the focus itself, on the day's own button (a roving tab
 * stop, as BigCalendar has it). It used to be a ring drawn on a day while the
 * focus stayed on the grid, and nothing told a screen reader which day that
 * was: the arrows moved silently, and a picker opened from the keyboard put
 * its focus on the month-paging button instead.
 */

export interface CalendarProps {
  /** The selected day, or `undefined` for none. */
  value?: IsoDate
  onValueChange?: (value: IsoDate) => void
  /** Which month is shown. Uncontrolled unless given. */
  month?: IsoDate
  onMonthChange?: (month: IsoDate) => void
  /** Bounds, inclusive. A day outside them cannot be chosen. */
  min?: IsoDate
  max?: IsoDate
  /** For a range: the other end, so the days between can be shaded. */
  rangeEnd?: IsoDate
  /** Formats the names. The application's language by default - see `useLocale`. */
  locale?: string
  /** What the grid is called, for a screen reader. */
  'aria-label'?: string
  /** Put the focus on the day under the cursor when it mounts - for a
   * calendar that opens in a popup, where choosing a day is what the reader
   * came for. Left to the popup, the focus went to the first thing in it: the
   * month-paging button, two Tabs away from the days. */
  autoFocus?: boolean
  /** Names the buttons that page the months. Required: they are icons, and an
   * icon with no name is a button that announces nothing. */
  previousMonthLabel: string
  nextMonthLabel: string
  className?: string
}

export function Calendar({
  value,
  onValueChange,
  month,
  onMonthChange,
  min,
  max,
  rangeEnd,
  locale,
  previousMonthLabel,
  nextMonthLabel,
  className,
  'aria-label': ariaLabel,
  autoFocus = false,
}: CalendarProps) {
  const [ownMonth, setOwnMonth] = useState<IsoDate>(() => value ?? today())
  const shown = month ?? ownMonth

  /* Which day the keyboard is on. It is not the selection: arrowing around a
   * calendar moves a cursor, and only Enter chooses - otherwise every arrow
   * key would fire `onValueChange` and a product listening for it would save
   * five dates on the way to the sixth. */
  const [focused, setFocused] = useState<IsoDate>(() => value ?? today())

  const language = useLocale(locale)
  const weeks = useMemo(() => monthGrid(shown, language), [shown, language])
  const start = firstDayOfWeek(language)
  const names = useMemo(() => weekdayNames(language, start), [language, start])
  const heading = useMemo(
    () => new Intl.DateTimeFormat(language, { month: 'long', year: 'numeric' }).format(
      new Date(parts(shown).year, parts(shown).month - 1, 1),
    ),
    [shown, language],
  )
  const dayNumber = useMemo(() => new Intl.DateTimeFormat(language, { day: 'numeric' }), [language])
  const fullDate = useMemo(
    () => new Intl.DateTimeFormat(language, { dateStyle: 'long' }),
    [language],
  )

  const outOfBounds = (date: IsoDate) =>
    (min !== undefined && date < min) || (max !== undefined && date > max)

  /* The day that takes the Tab: the cursor while it is on the grid, the
   * chosen day when the month buttons paged the cursor away, and the first of
   * the month when neither is shown - a grid with no tab stop cannot be
   * entered at all. */
  const days = useMemo(() => weeks.flat(), [weeks])
  const cursor = days.includes(focused)
    ? focused
    : value !== undefined && days.includes(value)
      ? value
      : days.find((date) => parts(date).month === parts(shown).month)!

  const grid = useRef<HTMLDivElement>(null)
  const refocus = useRef(autoFocus)
  useEffect(() => {
    if (!refocus.current) return
    refocus.current = false
    grid.current?.querySelector<HTMLElement>(`[data-day="${cursor}"]`)?.focus()
  })

  const goToMonth = (next: IsoDate) => {
    if (month === undefined) setOwnMonth(next)
    onMonthChange?.(next)
  }

  const moveFocus = (next: IsoDate) => {
    setFocused(next)
    refocus.current = true
    // Paging follows the cursor: arrowing off the end of a month shows the
    // next one rather than moving to a day nobody can see.
    if (parts(next).month !== parts(shown).month || parts(next).year !== parts(shown).year) {
      goToMonth(next)
    }
  }

  // Enter and Space are the focused day's own: a button clicks on both.
  const onKeyDown = (event: KeyboardEvent) => {
    const next = keyStep(event.key, cursor, start)
    if (next === undefined) return
    event.preventDefault()
    moveFocus(next)
  }

  const now = today()

  return (
    <div className={cn('w-64 select-none', className)}>
      <div className="mb-2 flex items-center justify-between gap-1">
        <button
          type="button"
          aria-label={previousMonthLabel}
          onClick={() => goToMonth(addMonths(shown, -1))}
          className={cn(
            'flex size-7 items-center justify-center rounded-md text-dim',
            'transition-colors hover:bg-soft hover:text-text',
            'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
          )}
        >
          <svg viewBox="0 0 16 16" className="size-4" aria-hidden>
            <path
              d="M10 3L5 8l5 5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>

        {/* The month is announced when it changes, so paging with the arrows
          * says where you have arrived rather than moving silently. */}
        <div aria-live="polite" className="text-sm font-medium text-text">
          {heading}
        </div>

        <button
          type="button"
          aria-label={nextMonthLabel}
          onClick={() => goToMonth(addMonths(shown, 1))}
          className={cn(
            'flex size-7 items-center justify-center rounded-md text-dim',
            'transition-colors hover:bg-soft hover:text-text',
            'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
          )}
        >
          <svg viewBox="0 0 16 16" className="size-4" aria-hidden>
            <path
              d="M6 3l5 5-5 5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>

      {/* One tab stop for the whole grid - the cursor's day - and the arrows
        * move within it. */}
      <div ref={grid} role="grid" aria-label={ariaLabel} onKeyDown={onKeyDown}>
        <div role="row" className="mb-1 grid grid-cols-7">
          {names.map((name) => (
            <div
              key={name}
              role="columnheader"
              aria-label={name}
              className="caption py-1 text-center"
            >
              {name}
            </div>
          ))}
        </div>

        {weeks.map((week) => (
          <div role="row" key={week[0]} className="grid grid-cols-7">
            {week.map((date) => {
              const outside = parts(date).month !== parts(shown).month
              const disabled = outOfBounds(date)
              const selected =
                value !== undefined &&
                (rangeEnd === undefined
                  ? date === value
                  : date === value || date === rangeEnd)
              const inRange =
                value !== undefined && rangeEnd !== undefined && date > value && date < rangeEnd

              return (
                <div role="gridcell" key={date} aria-selected={selected || undefined}>
                  <button
                    type="button"
                    data-day={date}
                    // A tab stop only under the cursor. Announced with its
                    // full date, because "14" on its own is not a date.
                    tabIndex={date === cursor ? 0 : -1}
                    // Not `disabled`: a disabled button cannot take the focus,
                    // and the cursor walks over days out of bounds on its way
                    // to the ones inside them.
                    aria-disabled={disabled || undefined}
                    aria-label={fullDate.format(new Date(parts(date).year, parts(date).month - 1, parts(date).day))}
                    aria-current={date === now ? 'date' : undefined}
                    // The day's state, said once on the element: the styles
                    // below read it, and so can a product's own or a test.
                    data-outside={outside || undefined}
                    data-selected={selected || undefined}
                    data-in-range={inRange || undefined}
                    onClick={() => {
                      setFocused(date)
                      if (!disabled) onValueChange?.(date)
                    }}
                    className={cn(
                      'flex h-8 w-full items-center justify-center rounded-md text-sm tabular-nums',
                      'transition-colors',
                      'text-text data-[outside]:text-faint',
                      'data-[in-range]:bg-accent-soft',
                      'data-[selected]:bg-accent data-[selected]:font-medium data-[selected]:text-on-accent',
                      'not-data-[selected]:not-aria-disabled:hover:bg-soft',
                      'not-data-[selected]:aria-[current=date]:font-medium not-data-[selected]:aria-[current=date]:text-accent',
                      'focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent',
                      'aria-disabled:cursor-not-allowed aria-disabled:opacity-40',
                    )}
                  >
                    {dayNumber.format(new Date(parts(date).year, parts(date).month - 1, parts(date).day))}
                  </button>
                </div>
              )
            })}
          </div>
        ))}
      </div>
    </div>
  )
}
