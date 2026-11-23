import { useId, useMemo, useState } from 'react'
import { cn, useLocale } from 'dowel-ui'
import { Button } from './button'
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
 *
 * Inside a `Field` the label names the trigger: the id and the two `aria-`
 * references Base UI hands a control are put on the button. Without them the
 * label's `for` pointed at nothing and the button was named by whatever date
 * it showed - kilna's own picker had grown the same three props for the same
 * reason.
 *
 * A date chosen by mistake in an optional field is taken back from the popup:
 * under the month, while a day is chosen, a button in the product's words. In
 * the popup rather than beside the trigger, because the trigger is a button
 * and a button cannot hold another, and a cross laid over the field is a
 * second, smaller target squeezed against the first. A required field gives no
 * words for it and cannot be emptied - and its `onValueChange` is never told
 * `undefined`, which the type says: only a picker given `clearLabel` takes a
 * handler that has to deal with nothing.
 */

/** Whether the date can be taken back, and what the handler is told. */
export type DatePickerClearing =
  | {
      /** The words of the button that takes the date back: "Clear". Leave it
       * out and a chosen date stays chosen - the shape of a required field. */
      clearLabel: string
      /** Told the chosen day, or `undefined` when it is taken back. */
      onValueChange?: (value: IsoDate | undefined) => void
    }
  | {
      clearLabel?: undefined
      /** Told the chosen day. */
      onValueChange?: (value: IsoDate) => void
    }

export type DatePickerProps = DatePickerClearing & {
  /** The chosen day, or `undefined` for none. */
  value?: IsoDate
  /** The earliest day that can be chosen, inclusive. */
  min?: IsoDate
  /** The latest day that can be chosen, inclusive. */
  max?: IsoDate
  /** What the trigger says when nothing is chosen. The product's word, since
   * a default here would be English inside a primitive. */
  placeholder: string
  /** Names the two month-paging buttons inside the calendar. */
  previousMonthLabel: string
  /** Names the next-month paging button inside the calendar. */
  nextMonthLabel: string
  /** How the date is written and which day starts the week. The
   * application's language unless stated - see `useLocale`. */
  locale?: string
  /** Stops the trigger opening the calendar. */
  disabled?: boolean
  /** Puts the value in a form: a hidden input of this name carries the date,
   * because the trigger is a button and a form would otherwise lose it. */
  name?: string
  /** Put on the trigger, so a `Field` label can point at it. */
  id?: string
  /** The accessible name of the field, in place of a visible label. */
  'aria-label'?: string
  /** Points at the element that names the field; a `Field` label hands this
   * over. The date shown on the trigger is added to the name. */
  'aria-labelledby'?: string
  /** Points at the hint or error text, put on the trigger. */
  'aria-describedby'?: string
  /** Marks the trigger as holding an invalid value. */
  'aria-invalid'?: boolean | 'true' | 'false'
  className?: string
}

export function DatePicker({
  value,
  onValueChange,
  clearLabel,
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
  id,
  'aria-labelledby': labelledBy,
  'aria-describedby': describedBy,
  'aria-invalid': invalid,
}: DatePickerProps) {
  const [open, setOpen] = useState(false)
  const language = useLocale(locale)
  // Either handler takes a day; only the one beside `clearLabel` is ever
  // handed nothing, and the button that does it is drawn only with the words.
  const report = onValueChange as ((value: IsoDate | undefined) => void) | undefined
  // Named by its label and then by the date it shows. A label alone would be
  // announced without the value on the button, and a name that does not hold
  // the words on the button fails WCAG 2.5.3: someone saying what they see to
  // voice control says words the name does not have. With an `aria-label` the
  // trigger names itself - an element listed in its own `aria-labelledby` is
  // read by its `aria-label`.
  const ownId = useId()
  const triggerId = id ?? ownId
  const shownId = useId()
  const naming = labelledBy ?? (ariaLabel === undefined ? undefined : triggerId)

  const shown = useMemo(() => {
    if (value === undefined || !isIsoDate(value)) return undefined
    const [year, month, day] = value.split('-').map(Number) as [number, number, number]
    return new Intl.DateTimeFormat(language, { dateStyle: 'long' }).format(
      new Date(year, month - 1, day),
    )
  }, [value, language])

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        disabled={disabled}
        aria-label={ariaLabel}
        id={triggerId}
        aria-labelledby={naming === undefined ? undefined : `${naming} ${shownId}`}
        aria-describedby={describedBy}
        aria-invalid={invalid}
        className={cn(
          fieldClasses,
          'flex h-control cursor-pointer items-center gap-2 text-left',
          'disabled:cursor-not-allowed',
          className,
        )}
      >
        <svg viewBox="0 0 16 16" className="size-4 shrink-0 text-faint" fill="none" aria-hidden>
          <rect x="2" y="3" width="12" height="11" rx="2" stroke="currentColor" strokeWidth="1.3" />
          <path d="M2 6.5h12M5.5 2v2M10.5 2v2" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
        </svg>
        {/* The empty face in `dim`: `faint` at this size measured 3.2:1. */}
        <span id={shownId} data-placeholder={shown === undefined ? '' : undefined} className="truncate data-[placeholder]:text-dim">
          {shown ?? placeholder}
        </span>
      </PopoverTrigger>

      {/* The value also goes into a form, because a button is not a field and
        * a form submitting the screen would otherwise lose the date. */}
      {name !== undefined && <input type="hidden" name={name} value={value ?? ''} />}

      {/* The calendar takes the focus to the day under the cursor itself, so
        * the popup moves it nowhere first. And the popup is a dialog, so it is
        * named - for the field. */}
      <PopoverPopup
        arrow={false}
        className="w-auto p-3"
        initialFocus={false}
        aria-label={labelledBy === undefined ? (ariaLabel ?? placeholder) : undefined}
        aria-labelledby={labelledBy}
      >
        <Calendar
          autoFocus
          value={value}
          min={min}
          max={max}
          locale={language}
          aria-label={ariaLabel ?? placeholder}
          previousMonthLabel={previousMonthLabel}
          nextMonthLabel={nextMonthLabel}
          onValueChange={(next) => {
            report?.(next)
            // Choosing a day is the whole errand: the popup closes rather
            // than waiting for a second dismissing click.
            setOpen(false)
          }}
        />
        {clearLabel !== undefined && value !== undefined && (
          <div className="mt-2 flex justify-end border-t border-line pt-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                report?.(undefined)
                setOpen(false)
              }}
            >
              {clearLabel}
            </Button>
          </div>
        )}
      </PopoverPopup>
    </Popover>
  )
}
