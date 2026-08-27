import { afterEach, describe, expect, it, vi } from 'vitest'
import { ApiError, api } from '@/lib/api'

/**
 * What is worth testing here is the failure path, not the happy one: a
 * successful call is proven by the app working, but a misread error turns
 * "wrong password" into "the server is down" on the login screen, and nothing
 * about the page looks broken when it does.
 */

function respond(status: number, body: unknown, ok = status < 400) {
  return {
    ok,
    status,
    statusText: `status ${status}`,
    json: () => Promise.resolve(body),
  } as Response
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('the API client', () => {
  it('reads the error the server sends', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(respond(401, { error: 'wrong email or password' })))

    const failure = await api.me().catch((error: unknown) => error)

    expect(failure).toBeInstanceOf(ApiError)
    expect((failure as ApiError).status).toBe(401)
    expect((failure as ApiError).message).toBe('wrong email or password')
    expect((failure as ApiError).isUnauthorized).toBe(true)
  })

  it('survives a failure body that is not the shape it expects', async () => {
    // A proxy or a crashed process answers with HTML, or with nothing at all.
    // Parsing that must not throw a second error on top of the first, hiding
    // the status that says what actually happened.
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: false,
        status: 502,
        statusText: 'Bad Gateway',
        json: () => Promise.reject(new SyntaxError('Unexpected token <')),
      } as unknown as Response),
    )

    const failure = (await api.me().catch((error: unknown) => error)) as ApiError

    expect(failure).toBeInstanceOf(ApiError)
    expect(failure.status).toBe(502)
    expect(failure.message).toBe('Bad Gateway')
    expect(failure.isUnauthorized).toBe(false)
  })

  it('sends the session cookie and the version-prefixed path', async () => {
    const fetchMock = vi.fn().mockResolvedValue(respond(200, { status: 'ok' }))
    vi.stubGlobal('fetch', fetchMock)

    await api.login('someone@example.test', 'secret')

    const [path, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(path).toBe('/api/v1/auth/login')
    // Without `credentials` the browser sends an anonymous request and every
    // page bounces to login, which looks like a server bug rather than a
    // missing option.
    expect(init.credentials).toBe('same-origin')
    expect(init.method).toBe('POST')
  })

  it('does not set a content type on a request with no body', async () => {
    const fetchMock = vi.fn().mockResolvedValue(respond(200, { level: 'full' }))
    vi.stubGlobal('fetch', fetchMock)

    await api.privacy()

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(init.headers).toEqual({})
  })
})
