import type { InputHTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

export function Input({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cn(
        'h-10 w-full rounded-[9px] border border-line bg-softer px-3 text-sm text-text',
        'placeholder:text-faint',
        // The focus ring is the accent, and it is a ring rather than a colour
        // swap: keyboard users need to see where they are without reading it.
        'outline-none focus-visible:border-accent focus-visible:ring-2 focus-visible:ring-accent-soft',
        'disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    />
  )
}
