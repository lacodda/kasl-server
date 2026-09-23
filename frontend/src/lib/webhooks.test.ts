import { describe, expect, it } from 'vitest'
import type { WebhookDelivery, WebhookDestination } from '@/lib/api'
import { deliveryState, deliveryTone, destinationProblem } from '@/lib/webhooks'

const delivery = (fields: Partial<WebhookDelivery>): WebhookDelivery => ({
  id: 'd',
  event_id: 'e',
  destination: 'team',
  event: 'alert.raised',
  person: 'Ana',
  created_at: '2026-09-23T08:00:00Z',
  attempts: 0,
  next_attempt_at: '2026-09-23T08:00:00Z',
  delivered_at: null,
  abandoned_at: null,
  last_status: null,
  last_error: null,
  ...fields,
})

const destination = (fields: Partial<WebhookDestination>): WebhookDestination => ({
  name: 'team',
  kind: 'slack',
  target: 'hooks.slack.com',
  events: ['alert.raised'],
  department: null,
  department_exists: null,
  pending: 0,
  delivered: 0,
  abandoned: 0,
  last_delivered_at: null,
  last_error: null,
  last_error_at: null,
  ...fields,
})

describe('deliveryState', () => {
  it('reads the outcome before the history', () => {
    // A message that failed twice and then went through is delivered. Reading
    // the attempt count first would draw it as still failing.
    expect(deliveryState(delivery({ attempts: 3, last_error: 'the receiver answered 503', delivered_at: '2026-09-23T08:10:00Z' }))).toBe('delivered')
    expect(deliveryState(delivery({ attempts: 9, last_error: 'gave up', abandoned_at: '2026-09-24T08:00:00Z' }))).toBe('abandoned')
  })

  it('tells a message not yet tried from one being retried', () => {
    expect(deliveryState(delivery({ attempts: 0 }))).toBe('queued')
    expect(deliveryState(delivery({ attempts: 1, last_error: 'connection refused' }))).toBe('retrying')
  })

  it('draws only the given-up ones as bad', () => {
    expect(deliveryTone('abandoned')).toBe('bad')
    expect(deliveryTone('retrying')).toBe('warn')
    expect(deliveryTone('delivered')).toBe('good')
  })
})

describe('destinationProblem', () => {
  it('puts a department nobody has above a failure', () => {
    // It hears nothing, so nothing will ever fail to say so. The screen is
    // the only place it can be noticed.
    expect(destinationProblem(destination({ department: 'Sales', department_exists: false, last_error: 'x' }))).toBe('departmentMissing')
  })

  it('names a failing destination and leaves a healthy one alone', () => {
    expect(destinationProblem(destination({ last_error: 'the receiver answered 404: no_service' }))).toBe('failing')
    expect(destinationProblem(destination({ department: 'Sales', department_exists: true }))).toBeNull()
    expect(destinationProblem(destination({}))).toBeNull()
  })
})
