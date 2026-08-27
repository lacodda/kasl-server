import type { ButtonHTMLAttributes } from 'react'
import { Slot } from '@radix-ui/react-slot'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '@/lib/utils'

export const buttonVariants = cva(
  'inline-flex cursor-pointer items-center justify-center gap-1.5 whitespace-nowrap font-medium transition-colors disabled:pointer-events-none disabled:opacity-50 [&_svg]:shrink-0',
  {
    variants: {
      variant: {
        primary: 'rounded-[10px] bg-accent font-semibold text-on-accent hover:bg-accent-2',
        ghost: 'rounded-[9px] border border-line text-dim hover:border-line-2 hover:text-text',
        soft: 'rounded-[9px] bg-accent-soft text-accent-2 hover:bg-accent/25',
        danger: 'rounded-[9px] text-bad hover:bg-bad-soft',
        icon: 'rounded-[9px] text-dim hover:bg-soft hover:text-text',
      },
      size: {
        sm: 'h-7 px-2.5 text-xs',
        md: 'h-9 px-3.5 text-sm',
        lg: 'h-10 px-4 text-sm',
        iconSm: 'size-7 [&_svg]:size-3.5',
        iconMd: 'size-8 [&_svg]:size-4',
      },
    },
    defaultVariants: { variant: 'ghost', size: 'md' },
  },
)

interface Props extends ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  asChild?: boolean
}

export function Button({ variant, size, asChild = false, className, type, ...props }: Props) {
  const Component = asChild ? Slot : 'button'
  return (
    <Component
      // A button inside a form defaults to `submit`, which surprises everyone
      // who adds one next to a text field. Ours says what it is.
      type={asChild ? type : (type ?? 'button')}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  )
}
