/**
 * The API client.
 *
 * These types mirror the server's response shapes. Nothing enforces that they
 * agree - the server is the contract, and a change there means a change here.
 *
 * Authentication is the session cookie from ADR 0007: the browser holds it, no
 * token is stored in JavaScript, and every request just carries it.
 */

export type UserRole = 'admin' | 'manager' | 'employee'

/** What `GET /auth/me` answers: the server's `Identity`. */
export interface Identity {
  id: string
  email: string
  display_name: string
  role: UserRole
}

export type PrivacyLevel = 'full' | 'moderate' | 'coarse'

export interface StoredItem {
  what: string
  detail: string
}

export interface PrivacyManifest {
  level: PrivacyLevel
  summary: string
  stored: StoredItem[]
  never_collected: string[]
  visible_to: string[]
  retention: string
  on_change: string
  updated_at: string | null
}

/** A request the server refused, carrying the status so a caller can branch. */
export class ApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }

  /** Not signed in, or the session ended. The caller shows the login screen. */
  get isUnauthorized() {
    return this.status === 401
  }
}

interface Options extends RequestInit {
  /** Path is from the site root rather than under `/api/v1` - only `/health`. */
  absolute?: boolean
}

async function request<T>(path: string, init?: Options): Promise<T> {
  const { absolute, ...rest } = init ?? {}
  const response = await fetch(absolute ? path : `/api/v1${path}`, {
    ...rest,
    // The session cookie. Without this the browser would send an anonymous
    // request and every page would bounce to login.
    credentials: 'same-origin',
    headers: {
      ...(rest.body ? { 'Content-Type': 'application/json' } : {}),
      ...rest.headers,
    },
  })

  if (!response.ok) {
    // The server answers a failure as `{"error": "..."}`, but a proxy or a
    // crash can return something else - so a body that is not the expected
    // shape must not turn into a second, confusing error.
    const message = await response
      .json()
      .then((body: unknown) =>
        typeof body === 'object' && body !== null && 'error' in body ? String(body.error) : response.statusText,
      )
      .catch(() => response.statusText)
    throw new ApiError(response.status, message)
  }

  // 204 and friends have no body to parse.
  if (response.status === 204) return undefined as T
  return (await response.json()) as T
}

export interface Health {
  status: string
  version: string
  database: string
}

export const api = {
  /**
   * The server's version, which is the product's version.
   *
   * Deliberately not the frontend's `package.json`: the UI is embedded in the
   * binary (ADR 0012), so showing the two separately would put two different
   * numbers on one product and leave a bug report naming the wrong one.
   */
  health: () => request<Health>('/health', { absolute: true }),

  /**
   * Signs in. The answer is only `{"status":"ok"}` - what matters is the
   * cookie it sets, so the caller follows with `me()` rather than reading a
   * user out of this response.
   */
  login: (email: string, password: string) =>
    request<{ status: string }>('/auth/login', { method: 'POST', body: JSON.stringify({ email, password }) }),

  logout: () => request<{ status: string }>('/auth/logout', { method: 'POST' }),

  me: () => request<Identity>('/auth/me'),

  privacy: () => request<PrivacyManifest>('/privacy'),
}
