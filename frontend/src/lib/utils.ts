import { clsx, type ClassValue } from 'clsx'
import { twMerge } from 'tailwind-merge'

/**
 * Joins class names and lets a later one win over an earlier conflicting
 * utility, which is what makes a `className` prop on a kit component able to
 * override the variant it was given.
 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
