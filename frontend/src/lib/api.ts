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

export interface Pause {
  id: string
  started_at: string
  ended_at: string | null
  duration_seconds: number | null
  /** A break the employee entered by hand, as opposed to detected idleness. */
  manual: boolean
  reason: string | null
}

export interface Task {
  id: string
  name: string
  comment: string | null
  completeness: number
  recorded_at: string
}

export interface Day {
  date: string
  started_at: string
  ended_at: string | null
  /** `null` while the day is still open - an unfinished day has no total. */
  worked_seconds: number | null
  paused_count: number
  paused_seconds: number
  pauses: Pause[]
  tasks: Task[]
}

/** What a privacy level withholds, in the server's own vocabulary. */
export type NotStored = 'pauses' | 'tasks' | 'free_text'

export interface DaysResponse {
  from: string
  to: string
  days: Day[]
  privacy_level: PrivacyLevel
  /**
   * Kinds of detail this installation does not keep. The screen must say so
   * where it would otherwise render an empty section: "no pauses stored" and
   * "you took no breaks" look identical, and only one of them is true.
   */
  not_stored: NotStored[]
}

/** One person's period, as the manager's dashboard lists them. */
export interface Member {
  id: string
  display_name: string
  email: string
  department: string | null
  days_recorded: number
  worked_seconds: number
  paused_seconds: number
  last_day: string | null
  /** A day is open right now on this person's calendar. */
  day_open: boolean
  /**
   * When one of their agents last delivered anything - the honest half of
   * "who is working now". The server knows when it last heard from a machine,
   * not whether someone is sitting at it.
   */
  last_seen_at: string | null
  /** Live agent tokens. Zero explains a silent row without guessing. */
  agents: number
}

export interface TeamResponse {
  from: string
  to: string
  members: Member[]
  privacy_level: PrivacyLevel
  not_stored: NotStored[]
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
  /**
   * This installation holds the demo's fictional team (ADR 0013). Read from
   * the database, not from the environment, so the label outlives the flag
   * that seeded it. Absent when the server could not reach its database.
   */
  demo?: boolean
}

/** One account a visitor may sign in as on a demo. */
export interface DemoAccount {
  role: UserRole
  email: string
  display_name: string
}

export interface DemoAccounts {
  /** The one password every demo account shares. */
  password: string
  accounts: DemoAccount[]
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

  /**
   * Who a visitor may sign in as. Answers only on a demo; anywhere else the
   * server says 404, so a real installation never lists its people to
   * someone who has not signed in.
   */
  demoAccounts: () => request<DemoAccounts>('/demo/accounts'),

  privacy: () => request<PrivacyManifest>('/privacy'),

  /**
   * The signed-in person's own days, both ends inclusive.
   *
   * Dates are the employee's local calendar dates as their agent recorded
   * them, not dates derived from a timestamp in the browser's zone (ADR 0003).
   */
  myDays: (from: string, to: string) => request<DaysResponse>(`/me/days?from=${from}&to=${to}`),

  /** The team's hours over a range. Managers and administrators only. */
  teamDays: (from: string, to: string) => request<TeamResponse>(`/team/days?from=${from}&to=${to}`),

  /**
   * One person's days, for a manager who may see them.
   *
   * Answers exactly what `myDays` does - the drill-down is the personal screen
   * pointed at someone else, so the two share a renderer.
   */
  userDays: (id: string, from: string, to: string) => request<DaysResponse>(`/users/${id}/days?from=${from}&to=${to}`),
}
