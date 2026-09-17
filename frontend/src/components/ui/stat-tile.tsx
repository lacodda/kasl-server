import type { HTMLAttributes, ReactNode } from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from 'dowel-ui'

/*
 * One figure, and what it is a figure of.
 *
 * The smallest thing on a dashboard and the one every product writes itself:
 * a label above, a number below, sometimes a word about which way it moved.
 * It is here because the first consumer had written it twice - once on the
 * personal page, once on the team one - and the copies had already drifted:
 * one had grown a warning tone the other lacked, and the tone classes in it
 * were concatenated without a space, so a figure that was both accented and
 * warning would have emitted `text-accent-2text-warn` and been styled by
 * neither. Nothing had gone wrong on screen yet; the two flags were simply
 * never passed together.
 *
 * A `<dl>` rather than two divs, for the reason KeyValue is one: the pairing
 * is what a screen reader announces. Loose divs read as two unrelated pieces
 * of text and nothing says the number belongs to the label.
 *
 * Numbers are set in the mono face with tabular figures, so a column of tiles
 * lines up and a value that ticks does not shuffle its neighbours sideways.
 * That matters more than it sounds: a live figure redrawn every few seconds in
 * proportional digits makes the whole row twitch.
 *
 * The delta is a second, quieter line rather than a colour on the value. A
 * number that turns red is a number whose colour has to be explained, and the
 * explanation is never on the screen; a delta says "+12% vs last week" and
 * needs nothing. Its tone is stated by the caller rather than inferred from
 * the sign, because down is good for a figure like "time to first response",
 * and a component cannot know which figure it is holding.
 */

export const statTileVariants = cva('min-w-0', {
  variants: {
    size: {
      /* The dashboard default: a row of these under a heading. */
      md: '',
      /* For a tile that leads a page rather than sitting in a row of six. */
      lg: '',
    },
  },
  defaultVariants: { size: 'md' },
})

export const statTileValueVariants = cva('mt-1 font-mono tabular-nums', {
  variants: {
    size: {
      md: 'text-lg',
      lg: 'text-2xl',
    },
    tone: {
      /* The reading tone: what this figure is, not how it is doing. `accent`
       * marks the one figure a panel is really about; `warn` and `bad` are for
       * a figure that is itself a problem - people with no agent reporting,
       * a queue that is backing up. */
      default: 'text-text',
      accent: 'text-accent-2',
      warn: 'text-warn',
      bad: 'text-bad',
    },
  },
  defaultVariants: { size: 'md', tone: 'default' },
})

export const statTileDeltaVariants = cva('mt-1 text-xs', {
  variants: {
    tone: {
      /* Neutral by default, because most movement is just movement. */
      default: 'text-dim',
      good: 'text-good',
      bad: 'text-bad',
    },
  },
  defaultVariants: { tone: 'default' },
})

export interface StatTileProps
  extends Omit<HTMLAttributes<HTMLDListElement>, 'title'>,
    VariantProps<typeof statTileVariants> {
  /** What the figure is. */
  label: ReactNode
  /** The figure. Already formatted - a duration, a count, a percentage: this
   * component decides how a number looks, never what it says. */
  value: ReactNode
  /** How the value itself reads. */
  tone?: NonNullable<VariantProps<typeof statTileValueVariants>['tone']>
  /** Which way it moved, in words the caller chooses: `+12% vs last week`,
   * `3 fewer than yesterday`. Omitted when there is nothing to compare to -
   * an empty line here reads as "unchanged", which is a claim. */
  delta?: ReactNode
  /** Whether that movement is good news. Stated rather than read off the sign,
   * because for a figure like time-to-answer a fall is the good direction. */
  deltaTone?: NonNullable<VariantProps<typeof statTileDeltaVariants>['tone']>
}

export function StatTile({
  label,
  value,
  tone,
  delta,
  deltaTone,
  size,
  className,
  ...props
}: StatTileProps) {
  return (
    <dl className={cn(statTileVariants({ size }), className)} {...props}>
      <dt className="text-xs font-medium text-dim">{label}</dt>
      <dd className={cn(statTileValueVariants({ size, tone }))}>{value}</dd>
      {/* A second `dd` for the same term: the spec allows several, and this is
       * what they are for - one fact with two parts. A `<div>` here would end
       * the description list's pairing, and the delta would be read as loose
       * text next to the number rather than as part of it. */}
      {delta !== undefined && delta !== null && (
        <dd className={cn(statTileDeltaVariants({ tone: deltaTone }))}>{delta}</dd>
      )}
    </dl>
  )
}

/*
 * A row of tiles.
 *
 * Both donors wrote the identical container - `flex flex-wrap items-baseline
 * gap-x-8 gap-y-3` inside a Panel - and both had to get `items-baseline`
 * right, which is the part that is easy to miss: without it, a tile carrying a
 * delta is taller than its neighbours and the whole row's numbers stop sharing
 * a line.
 *
 * Wrapping rather than a grid, because the number of figures is decided at
 * runtime (one donor hides two of its five until there is something to say),
 * and a grid with a fixed column count leaves a hole where a hidden tile was.
 */
export type StatRowProps = HTMLAttributes<HTMLDivElement>

export function StatRow({ className, ...props }: StatRowProps) {
  return <div className={cn('flex flex-wrap items-baseline gap-x-8 gap-y-3', className)} {...props} />
}
