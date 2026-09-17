import type { HTMLAttributes, ReactNode } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from 'dowel-ui'

/*
 * Columns over a baseline: one period, one bar.
 *
 * Bars rather than a line, and the distinction is the data's not the
 * drawing's: a line says the value exists between the points, a column says
 * each period is its own sum. Hours worked in a week is a sum - there is no
 * "Wednesday afternoon" reading between two weeks - so it is a column.
 *
 * **The chart owns its height in pixels.** That is not a style choice, it is
 * the defect this component exists to prevent. The first consumer drew its
 * bars as a percentage of the parent, inside a flex row sized from its own
 * content: the child had no base to be a percentage of, every bar computed to
 * zero, and the chart shipped as a row of bare axis labels. No test and no API
 * check could see it - the owner found it by looking, and it cost a patch
 * release. Here the plot is a stated number of pixels and a bar is a
 * percentage of *that*.
 *
 * **An absent period is not a short one.** A week with nothing recorded and a
 * week of twenty minutes are different facts, and a bar of no height says
 * neither - it reads as a bar that failed to render. So a gap keeps its place
 * in the row and is drawn as a mark of its own.
 *
 * Interaction is one `title` per column rather than a tooltip layer: this is a
 * small chart that sits inside a panel, and the numbers it holds also belong
 * in the list beside it. A reader who needs them exactly should not have to
 * hover to find out.
 */

export const barChartVariants = cva('flex items-end gap-1 border-b border-chart-axis', {
  variants: {
    size: {
      /* Inside a panel, beside other things. */
      sm: '[--plot:72px]',
      /* On its own, where the shape is the subject. */
      md: '[--plot:112px]',
    },
  },
  defaultVariants: { size: 'md' },
})

export const barVariants = cva('mx-auto w-3/5 max-w-6 rounded-t-[4px]', {
  variants: {
    tone: {
      accent: 'bg-accent',
      /* For a bar that is one of several series, or one the caller is
       * highlighting against the rest. */
      series: 'bg-series-1',
      good: 'bg-good',
      warn: 'bg-warn',
      bad: 'bg-bad',
      /* The rest of the field, when one bar is the story: emphasis is the
       * most underused form in a chart of eight equal colours. */
      muted: 'bg-line-2',
    },
  },
  defaultVariants: { tone: 'accent' },
})

export interface BarDatum {
  /** Distinguishes this column from its neighbours. */
  key: string
  /** The period's own sum. `null` is a period with nothing recorded, which is
   * not the same as a sum of zero and is not drawn as a bar. */
  value: number | null
  /** What goes under the column. Kept short - these labels sit at a width the
   * chart does not control. */
  label: ReactNode
  tone?: NonNullable<VariantProps<typeof barVariants>['tone']>
  /** What the column says, for a reader hovering it and for a screen reader.
   * Required per bar, because a rectangle announces nothing. */
  title: string
}

export interface BarChartProps
  extends Omit<HTMLAttributes<HTMLDivElement>, 'children'>,
    VariantProps<typeof barChartVariants> {
  bars: BarDatum[]
  /** The top of the scale. Defaults to the tallest bar present; state it to
   * compare two charts, or to hold a scale still while the data moves. */
  max?: number
  /** What the whole chart is, in words. */
  label: string
}

/** The shortest a bar may be drawn, as a percentage of the plot.
 *
 * A twenty-minute week against a forty-hour one is half a percent, which
 * rounds to nothing: the period was recorded and would simply not be there. */
const MIN_BAR = 3

export function BarChart({ bars, max, label, size, className, ...props }: BarChartProps) {
  const values = bars.map((bar) => bar.value).filter((value): value is number => value !== null)
  /* A ceiling of at least one, so a chart of nothing but zeroes divides
   * safely and draws a row of floors rather than nothing at all. */
  const ceiling = Math.max(max ?? 0, ...values, 1)

  return (
    <div className={cn(barChartVariants({ size }), className)} role="img" aria-label={label} {...props}>
      {bars.map((bar) => (
        <div key={bar.key} className="flex min-w-0 flex-1 flex-col items-center gap-1.5">
          {/* The track carries the height itself. See the note above: a
            * percentage against a parent that has no height of its own
            * resolves to zero, and the chart disappears without failing. */}
          <div className="flex h-[var(--plot)] w-full items-end" title={bar.title}>
            {bar.value === null ? (
              /* A gap, drawn as one. A bar of no height is indistinguishable
               * from a bar that did not render, and closing the gap up would
               * turn an absence into continuity - the one thing a trend must
               * not do. */
              <div className="mx-auto h-1 w-3/5 max-w-6 rounded-t-[4px] border-x border-t border-line-2" />
            ) : (
              <div
                className={cn(barVariants({ tone: bar.tone }))}
                style={{ height: `${Math.max((bar.value / ceiling) * 100, MIN_BAR)}%` }}
              />
            )}
          </div>
          <span className="w-full truncate text-center text-[10px] text-faint tabular-nums">
            {bar.label}
          </span>
        </div>
      ))}
    </div>
  )
}

/*
 * A line across the plot, at a value on the same scale.
 *
 * For the number every bar is read against - a median, a target, an agreed
 * norm. A chart without its baseline invites the reader to invent one, and the
 * one they invent is usually the tallest bar.
 *
 * Drawn as a solid hairline rather than a dash: a dashed rule reads as
 * "projected" or "threshold" when it is neither, and the doctrine is explicit
 * that grid and axis lines are solid.
 */
export interface BaselineProps extends HTMLAttributes<HTMLDivElement> {
  /** Where it sits, on the same scale as the bars. */
  value: number
  /** The same ceiling the chart is drawn against. */
  max: number
  /** What the line is, beside it. */
  children?: ReactNode
}

export function Baseline({ value, max, children, className, ...props }: BaselineProps) {
  if (!(max > 0) || value < 0 || value > max) return null

  return (
    <div
      className={cn('pointer-events-none absolute inset-x-0 flex items-center gap-2', className)}
      style={{ bottom: `${(value / max) * 100}%` }}
      {...props}
    >
      {/* The rule stops short of the label rather than running under it: a
        * hairline crossing its own caption reads as a strikethrough. */}
      <div className="h-px flex-1 bg-chart-grid" />
      {children !== undefined && (
        <span className="shrink-0 text-[10px] leading-none text-faint tabular-nums">{children}</span>
      )}
    </div>
  )
}

/*
 * The box a chart and its baseline share.
 *
 * Two things it does, and both are the component's job rather than the
 * caller's. It establishes the positioning context the baseline needs - left
 * to the caller that is a `relative` remembered or forgotten, and forgotten it
 * puts the rule at the bottom of the page. And it keeps a gutter on the right
 * for the baseline's label, which otherwise sits on top of the last columns:
 * the label is outside the plot, so the plot has to end before it starts.
 */
export interface ChartFrameProps extends HTMLAttributes<HTMLDivElement> {
  /** Room on the right for the baseline's label. Omit it where there is no
   * baseline, or where the label is short enough to live in the panel's own
   * padding. */
  gutter?: boolean
  children: ReactNode
}

export function ChartFrame({ gutter = true, className, ...props }: ChartFrameProps) {
  return <div className={cn('relative', gutter && 'pr-16', className)} {...props} />
}
