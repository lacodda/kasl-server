/**
 * Where a signed-in reader may go.
 *
 * One list, drawn twice: as tabs in the header from `sm` up, and as the bar
 * along the bottom of a phone. Separate lists would be two places to add a
 * screen to, and forgetting the second one makes that screen unreachable on a
 * phone without anything failing - the header simply is not there to notice.
 *
 * Here rather than beside the component because it is the part worth a test:
 * who sees what, and in what order.
 */
export interface Destination {
  to: string
  /** The translation key. Short names - these are tabs, not headings: the
   * privacy screen is called "What is stored about you" and truncated to
   * "What is stored a…" in a bar four cells wide. */
  label: string
  /** Named rather than imported, so this file stays free of React. */
  icon: 'day' | 'team' | 'month' | 'privacy'
}

/**
 * The destinations, in the order both navigations show them.
 *
 * The team screens are left out for someone the server would refuse anyway -
 * this is tidiness, not a guard. Every one of those routes answers 403 on its
 * own.
 */
export function destinations(managesPeople: boolean): Destination[] {
  return [
    { to: '/day', label: 'nav.myDay', icon: 'day' },
    ...(managesPeople
      ? ([
          { to: '/team', label: 'nav.team', icon: 'team' },
          { to: '/month', label: 'nav.heatmap', icon: 'month' },
        ] as const)
      : []),
    { to: '/privacy', label: 'nav.privacy', icon: 'privacy' },
  ]
}
