/**
 * Reading the delivery log.
 *
 * Out of the component for the reason `alertPhrase` is: which state a row is
 * in is the part that can be wrong in a way a screenshot hides - a delivery
 * the server gave up on, drawn as "sending", reads perfectly and is a lie -
 * and a function can be asserted where a row of markup cannot.
 */

import type { WebhookDelivery, WebhookDestination } from '@/lib/api'

/**
 * Where one delivery stands.
 *
 * - `delivered` - the receiver took it.
 * - `queued` - not tried yet; the dispatcher looks every few seconds.
 * - `retrying` - tried and failed, and will be tried again at `next_attempt_at`.
 * - `abandoned` - given up on: refused, out of retries, or its destination
 *   was removed from the environment.
 */
export type DeliveryState = 'delivered' | 'queued' | 'retrying' | 'abandoned'

export function deliveryState(delivery: WebhookDelivery): DeliveryState {
  // The two outcomes first. A row carries an attempt count and an error from
  // before it succeeded or was given up on, and reading those before the
  // outcome would draw a delivered message as a failing one.
  if (delivery.delivered_at) return 'delivered'
  if (delivery.abandoned_at) return 'abandoned'
  return delivery.attempts === 0 ? 'queued' : 'retrying'
}

/** How a state is drawn. `bad` only for what will not happen on its own. */
export function deliveryTone(state: DeliveryState): 'good' | 'info' | 'warn' | 'bad' {
  switch (state) {
    case 'delivered':
      return 'good'
    case 'queued':
      return 'info'
    case 'retrying':
      return 'warn'
    case 'abandoned':
      return 'bad'
  }
}

/**
 * What is wrong with a destination as configured, if anything, as an i18n
 * key. One answer, the most serious: a screen listing three problems with one
 * destination buries the one that explains the other two.
 *
 * - `departmentMissing` - it names a department nobody has, so it hears
 *   nothing at all. Worse than a failure, because nothing ever fails.
 * - `failing` - its most recent delivery in flight or given up on carries an
 *   error.
 */
export function destinationProblem(destination: WebhookDestination): 'departmentMissing' | 'failing' | null {
  if (destination.department_exists === false) return 'departmentMissing'
  if (destination.last_error) return 'failing'
  return null
}
