import type { HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

/** A raised surface: the card every screen is built out of. */
export function Panel({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn('rounded-xl border border-line bg-raise', className)} {...props} />
}
