import { describe, expect, it } from 'vitest'
import type { ReadToken } from '@/lib/api'
import { datasetUrl, tokenState } from '@/lib/read-tokens'

const token = (fields: Partial<ReadToken>): ReadToken => ({
  id: 't',
  name: 'Grafana',
  scope: 'hours',
  owner_id: 'u',
  owner: 'Ana',
  created_at: '2026-10-20T08:00:00Z',
  last_used_at: null,
  revoked_at: null,
  ...fields,
})

describe('datasetUrl', () => {
  it('gives a ranged dataset the time picker, in the form the server reads', () => {
    expect(datasetUrl('https://kasl.example.com', 'days')).toBe(
      'https://kasl.example.com/api/bi/v1/days?from=${__from:date:YYYY-MM-DD}&to=${__to:date:YYYY-MM-DD}',
    )
  })

  it('asks for no range where the server takes none', () => {
    expect(datasetUrl('https://kasl.example.com', 'people')).toBe('https://kasl.example.com/api/bi/v1/people')
  })

  it('puts the CSV extension on the path, before the range', () => {
    expect(datasetUrl('http://pi:8082/', 'summary', 'csv')).toBe(
      'http://pi:8082/api/bi/v1/summary.csv?from=${__from:date:YYYY-MM-DD}&to=${__to:date:YYYY-MM-DD}',
    )
  })
})

describe('tokenState', () => {
  it('reads a revocation before any use', () => {
    // Read with every minute until somebody revoked it: it reads nothing now.
    expect(tokenState(token({ last_used_at: '2026-10-20T09:00:00Z', revoked_at: '2026-10-20T10:00:00Z' }))).toBe('revoked')
  })

  it('tells a token nothing has read with from one in use', () => {
    expect(tokenState(token({}))).toBe('unused')
    expect(tokenState(token({ last_used_at: '2026-10-20T09:00:00Z' }))).toBe('live')
  })
})
