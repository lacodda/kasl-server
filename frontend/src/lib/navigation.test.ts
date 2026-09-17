import { describe, expect, it } from 'vitest'
import { destinations } from '@/lib/navigation'
import en from '@/i18n/locales/en.json'

/*
 * The navigation.
 *
 * What these guard is the failure that says nothing when it happens: a screen
 * reachable in one navigation and not the other. On a desktop the header's
 * tabs are there to notice; on a phone there is only the bottom bar, and a
 * destination missing from it is a screen that cannot be opened at all.
 */

const resolve = (key: string): string | undefined =>
  key.split('.').reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], en) as
    | string
    | undefined

describe('destinations', () => {
  it('gives an employee their own screens and nothing else', () => {
    expect(destinations(false).map((d) => d.to)).toEqual(['/day', '/privacy'])
  })

  it('gives a manager the team screens as well, in reading order', () => {
    expect(destinations(true).map((d) => d.to)).toEqual(['/day', '/team', '/month', '/privacy'])
  })

  it('names every destination with a string the product actually has', () => {
    // A missing key renders as the key itself: `nav.myDay` in the tab bar,
    // which looks like a bug nobody filed rather than like a missing string.
    for (const destination of destinations(true)) {
      expect(resolve(destination.label), destination.label).toBeTypeOf('string')
    }
  })

  it('keeps the labels short enough for a bar four cells wide', () => {
    // At 390px a cell is about 90px, which holds roughly twelve characters at
    // 11px. The screen headings are longer than that on purpose - "What is
    // stored about you" - and this is what keeps one from being used as a tab
    // name again.
    for (const destination of destinations(true)) {
      expect(resolve(destination.label)!.length, destination.label).toBeLessThanOrEqual(12)
    }
  })

  it('leads every destination somewhere different', () => {
    const routes = destinations(true).map((d) => d.to)
    expect(new Set(routes).size).toBe(routes.length)
  })
})
