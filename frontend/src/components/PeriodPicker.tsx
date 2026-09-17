import type { ReactNode } from 'react'
import { ChevronLeft, ChevronRight } from 'lucide-react'
import { Button } from '@/components/ui/button'

/**
 * Back, here, forward - the control every screen with a period has.
 *
 * One component rather than the three copies the screens had grown, because
 * the phone layout is the part that is easy to get subtly wrong and easy to
 * fix in only two places: the buttons are the minimum touch size below `sm`
 * and the row spans the width so the arrows sit at the thumbs rather than
 * bunched in a corner.
 *
 * The middle button is a word the caller chooses - "This week", "This month" -
 * because a screen that pages by month must not offer to jump to this week.
 */
export function PeriodPicker({
  previousLabel,
  nextLabel,
  onPrevious,
  onNext,
  onNow,
  children,
}: {
  previousLabel: string
  nextLabel: string
  onPrevious: () => void
  onNext: () => void
  onNow: () => void
  /** What the middle button says: the name of the period it returns to. */
  children: ReactNode
}) {
  return (
    <div className="flex items-center justify-between gap-1.5 sm:justify-end">
      {/* `icon-md` is 32px, under the 44px a finger needs. The size is lifted
          on a phone and handed back at `sm`, where the pointer is a mouse and
          a row of three large buttons would shout. */}
      <Button variant="icon" size="icon-md" className="size-11 sm:size-8" aria-label={previousLabel} onClick={onPrevious}>
        <ChevronLeft />
      </Button>
      {/* Wide enough to be an easy target, not so wide that it becomes the
          screen's main action: stretched across the row it read as the thing
          to press, when the thing to press is usually an arrow. */}
      <Button size="sm" className="h-11 px-6 sm:h-7 sm:px-2.5" onClick={onNow}>
        {children}
      </Button>
      <Button variant="icon" size="icon-md" className="size-11 sm:size-8" aria-label={nextLabel} onClick={onNext}>
        <ChevronRight />
      </Button>
    </div>
  )
}
