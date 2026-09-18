import { useMemo, useState, type KeyboardEvent } from 'react'
import { cn } from 'dowel-ui'
import {
  addDays,
  addMonths,
  firstDayOfWeek,
  monthGrid,
  parts,
  today,
  weekday,
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
  /** Formats the names. Left alone it is the reader's own. */
  locale?: string
  /** What the grid is called, for a screen reader. */
  'aria-label'?: string
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
}: CalendarProps) {
  const [ownMonth, setOwnMonth] = useState<IsoDate>(() => value ?? today())
  const shown = month ?? ownMonth

  /* Which day the keyboard is on. It is not the selection: arrowing around a
   * calendar moves a cursor, and only Enter chooses - otherwise every arrow
   * key would fire `onValueChange` and a product listening for it would save
   * five dates on the way to the sixth. */
  const [focused, setFocused] = useState<IsoDate>(() => value ?? today())

  const weeks = useMemo(() => monthGrid(shown, locale), [shown, locale])
  const start = firstDayOfWeek(locale)
  const names = useMemo(() => weekdayNames(locale, start), [locale, start])
  const heading = useMemo(
    () => new Intl.DateTimeFormat(locale, { month: 'long', year: 'numeric' }).format(
      new Date(parts(shown).year, parts(shown).month - 1, 1),
    ),
    [shown, locale],
  )
  const dayNumber = useMemo(() => new Intl.DateTimeFormat(locale, { day: 'numeric' }), [locale])
  const fullDate = useMemo(
    () => new Intl.DateTimeFormat(locale, { dateStyle: 'long' }),
    [locale],
  )

  const outOfBounds = (date: IsoDate) =>
    (min !== undefined && date < min) || (max !== undefined && date > max)

  const goToMonth = (next: IsoDate) => {
    if (month === undefined) setOwnMonth(next)
    onMonthChange?.(next)
  }

  const moveFocus = (next: IsoDate) => {
    setFocused(next)
    // Paging follows the cursor: arrowing off the end of a month shows the
    // next one rather than moving to a day nobody can see.
    if (parts(next).month !== parts(shown).month || parts(next).year !== parts(shown).year) {
      goToMonth(next)
    }
  }

  const onKeyDown = (event: KeyboardEvent) => {
    const jump: Record<string, () => IsoDate> = {
      ArrowRight: () => addDays(focused, 1),
      ArrowLeft: () => addDays(focused, -1),
      ArrowDown: () => addDays(focused, 7),
      ArrowUp: () => addDays(focused, -7),
      PageDown: () => addMonths(focused, 1),
      PageUp: () => addMonths(focused, -1),
      Home: () => addDays(focused, -((weekday(focused) - start + 7) % 7)),
      End: () => addDays(focused, 6 - ((weekday(focused) - start + 7) % 7)),
    }

    const move = jump[event.key]
    if (move) {
      event.preventDefault()
      moveFocus(move())
      return
    }

    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      if (!outOfBounds(focused)) onValueChange?.(focused)
    }
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

      {/* One tab stop for the whole grid, and the arrows move within it - the
        * arrangement a radio group has, and the reason a calendar is usable at
        * all: forty-two tab stops is not a control. */}
      <div
        role="grid"
        aria-label={ariaLabel}
        tabIndex={0}
        onKeyDown={onKeyDown}
        className={cn(
          'rounded-md',
          'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent',
        )}
      >
        <div role="row" className="mb-1 grid grid-cols-7">
          {names.map((name) => (
            <div
              key={name}
              role="columnheader"
              aria-label={name}
              className="py-1 text-center text-2xs uppercase tracking-caption text-faint"
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
                    // Not a tab stop: the grid is the control. Announced with
                    // its full date, because "14" on its own is not a date.
                    tabIndex={-1}
                    disabled={disabled}
                    aria-label={fullDate.format(new Date(parts(date).year, parts(date).month - 1, parts(date).day))}
                    aria-current={date === now ? 'date' : undefined}
                    onClick={() => {
                      setFocused(date)
                      if (!disabled) onValueChange?.(date)
                    }}
                    className={cn(
                      'flex h-8 w-full items-center justify-center rounded-md text-sm tabular-nums',
                      'transition-colors',
                      outside ? 'text-faint' : 'text-text',
                      inRange && 'bg-accent-soft',
                      selected && 'bg-accent font-medium text-on-accent',
                      !selected && !disabled && 'hover:bg-soft',
                      date === now && !selected && 'font-medium text-accent',
                      date === focused && 'ring-1 ring-line-2',
                      disabled && 'cursor-not-allowed opacity-40',
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
