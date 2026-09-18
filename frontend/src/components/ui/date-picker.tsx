import { useMemo, useState } from 'react'
import { cn } from 'dowel-ui'
import { fieldClasses } from './input'
import { Popover, PopoverPopup, PopoverTrigger } from './popover'
import { Calendar } from './calendar'
import { isIsoDate, type IsoDate } from './calendar-math'

/*
 * DatePicker - a field that opens a month.
 *
 * The trigger is a button rather than a text input, and that is the decision
 * worth stating. A typable date field has to answer "what does `03/04/26`
 * mean" in a locale it cannot be sure of, and it answers wrong for half the
 * world; a button showing the date spelled out has no such question. Where
 * typing genuinely matters - a birth date, forty years back - the calendar is
 * the wrong control anyway and a product should reach for a plain field.
 *
 * The value is a calendar date as a string, `YYYY-MM-DD`, for the reasons the
 * Calendar states: a date with a timezone is a moment, and moments cross
 * midnight when they are serialised.
 *
 * What is shown is `Intl`'s own long form - "2 September 2026" here, "September
 * 2, 2026" in the United States - because a date written the reader's way is
 * one they do not have to decode.
 */

export interface DatePickerProps {
  /** The chosen day, or `undefined` for none. */
  value?: IsoDate
  onValueChange?: (value: IsoDate) => void
  /** Bounds, inclusive. */
  min?: IsoDate
  max?: IsoDate
  /** What the trigger says when nothing is chosen. The product's word, since
   * a default here would be English inside a primitive. */
  placeholder: string
  /** Names the two month-paging buttons inside the calendar. */
  previousMonthLabel: string
  nextMonthLabel: string
  /** How the date is written and which day starts the week. The reader's own
   * unless stated. */
  locale?: string
  disabled?: boolean
  name?: string
  'aria-label'?: string
  className?: string
}

export function DatePicker({
  value,
  onValueChange,
  min,
  max,
  placeholder,
  previousMonthLabel,
  nextMonthLabel,
  locale,
  disabled = false,
  name,
  className,
  'aria-label': ariaLabel,
}: DatePickerProps) {
  const [open, setOpen] = useState(false)

  const shown = useMemo(() => {
    if (value === undefined || !isIsoDate(value)) return undefined
    const [year, month, day] = value.split('-').map(Number) as [number, number, number]
    return new Intl.DateTimeFormat(locale, { dateStyle: 'long' }).format(
      new Date(year, month - 1, day),
    )
  }, [value, locale])

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        disabled={disabled}
        aria-label={ariaLabel}
        className={cn(
          fieldClasses,
          'flex h-9 cursor-pointer items-center gap-2 text-left',
          'disabled:cursor-not-allowed',
          className,
        )}
      >
        <svg viewBox="0 0 16 16" className="size-4 shrink-0 text-faint" fill="none" aria-hidden>
          <rect x="2" y="3" width="12" height="11" rx="2" stroke="currentColor" strokeWidth="1.3" />
          <path d="M2 6.5h12M5.5 2v2M10.5 2v2" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
        </svg>
        <span className={cn('truncate', shown === undefined && 'text-faint')}>
          {shown ?? placeholder}
        </span>
      </PopoverTrigger>

      {/* The value also goes into a form, because a button is not a field and
        * a form submitting the screen would otherwise lose the date. */}
      {name !== undefined && <input type="hidden" name={name} value={value ?? ''} />}

      <PopoverPopup arrow={false} className="w-auto p-3">
        <Calendar
          value={value}
          min={min}
          max={max}
          locale={locale}
          aria-label={ariaLabel ?? placeholder}
          previousMonthLabel={previousMonthLabel}
          nextMonthLabel={nextMonthLabel}
          onValueChange={(next) => {
            onValueChange?.(next)
            // Choosing a day is the whole errand: the popup closes rather
            // than waiting for a second dismissing click.
            setOpen(false)
          }}
        />
      </PopoverPopup>
    </Popover>
  )
}
