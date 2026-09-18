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

/**
 * What kind of day this was, as the employee's agent reported it.
 *
 * A day the agent said nothing about is `work` - every kasl shipped before the
 * field exists says so by saying nothing (ADR 0017).
 */
export type WorkdayKind = 'work' | 'vacation' | 'sick' | 'day_off'

export interface Day {
  date: string
  kind: WorkdayKind
  started_at: string
  ended_at: string | null
  /** `null` while the day is still open - an unfinished day has no total. */
  worked_seconds: number | null
  paused_count: number
  paused_seconds: number
  pauses: Pause[]
  tasks: Task[]
  /**
   * What this date was meant to be worked, in seconds. Zero on a weekend, a
   * holiday, and a day the employee was away.
   */
  norm_seconds: number
}

/** What a privacy level withholds, in the server's own vocabulary. */
export type NotStored = 'pauses' | 'tasks' | 'free_text'

/**
 * What a range asked of a person.
 *
 * A pair rather than a percentage: the server answers the two numbers a person
 * reads, and the screen divides them if it wants to (ADR 0017).
 */
export interface Progress {
  norm_seconds: number
  /** The installation's full day, in hours. */
  standard_hours: number
  /** This person's share of it: `0.5` is half time. */
  work_rate: number
}

export interface DaysResponse {
  from: string
  to: string
  days: Day[]
  progress: Progress
  /** Seconds worked across the range - what the norm is measured against. */
  worked_seconds: number
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
  /** Days worked in the range. Days away are counted separately below. */
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
  /**
   * This person's share of a full day. Carried so a row at half the team's
   * hours reads as half time rather than as half-hearted.
   */
  work_rate: number
  /** What the range asked of them, with days away taken out. */
  norm_seconds: number
  /** Days in the range they were on leave or ill. */
  days_away: number
}

export interface TeamResponse {
  from: string
  to: string
  members: Member[]
  privacy_level: PrivacyLevel
  not_stored: NotStored[]
  /** The installation's full day, in hours: one figure for the whole table. */
  standard_hours: number
}

/** What makes a date unlike the weekday it falls on. */
export type CalendarDayKind = 'holiday' | 'short_day' | 'working_weekend'

export interface CalendarDay {
  date: string
  kind: CalendarDayKind
  /** What the day is called. The date and the kind are the calendar. */
  note: string | null
}

export interface CalendarYear {
  year: number
  /** Only the exceptions, ascending. An empty year is a real answer. */
  days: CalendarDay[]
  standard_hours: number
}

/**
 * What the server concludes about a person right now.
 *
 * The first three are what an agent claimed. `offline` and `unknown` are not
 * claims - they are what silence means, and the two are kept apart on purpose:
 * an agent that stopped sending is a different fact from one that never sent
 * anything, and only the first says something about the person.
 */
export type LiveStatus = 'working' | 'paused' | 'idle' | 'offline' | 'unknown'

export interface LiveMember {
  user_id: string
  status: LiveStatus
  /** Seconds since the pulse arrived; null when none ever has. */
  since_received: number | null
}

export interface LiveTeamResponse {
  members: LiveMember[]
  /** How often to ask again. The server owns the cadence, not this client. */
  poll_seconds: number
  stale_after_seconds: number
}

/** One day of one person's month, as the heatmap draws it. */
export interface HeatmapCell {
  date: string
  /**
   * `null` for a day still open - it has no total yet. A cell that is absent
   * from the row is different again: nothing was recorded on that date, and
   * the grid must not draw the two the same way.
   */
  worked_seconds: number | null
  open: boolean
}

/** One person's month. */
export interface HeatmapRow {
  user_id: string
  display_name: string
  department: string | null
  /** Only the dates with a workday, ascending. Empty is a real answer. */
  days: HeatmapCell[]
  /** The longest finished day in this row; `null` when there is none. */
  busiest_seconds: number | null
  worked_seconds: number
}

export interface HeatmapResponse {
  month: string
  from: string
  to: string
  rows: HeatmapRow[]
  /** The busiest day anywhere in the grid - the scale every cell shares. */
  busiest_seconds: number | null
}

/**
 * What the server noticed about one person.
 *
 * Always about that person against their own history - never against a
 * colleague and never against a norm, which this server does not have.
 */
export type SignalKind = 'declining' | 'no_data' | 'unusual_week'

export interface Signal {
  user_id: string
  display_name: string
  department: string | null
  kind: SignalKind
  /** Weeks the slide has been running. `declining` only. */
  weeks: number | null
  /** Where the slide started, in seconds of a week. `declining` only. */
  from_seconds: number | null
  /** Where it stands now. `declining` and `unusual_week`. */
  to_seconds: number | null
  /** The person's own median week - what `unusual_week` is unusual against. */
  median_seconds: number | null
  /** Days since the last recorded day. `no_data` only. */
  days_quiet: number | null
}

export interface SignalsResponse {
  from: string
  to: string
  signals: Signal[]
  /**
   * People examined. "Nothing found among twelve" and "nothing found because
   * nobody was looked at" are different answers, and a screen that cannot tell
   * them apart shows the reassuring one.
   */
  people: number
}

/** One week on the trend chart. */
export interface TrendWeek {
  week_start: string
  worked_seconds: number
  /** Zero says the silence is real rather than a week of very short days. */
  days_recorded: number
}

export interface TrendResponse {
  user_id: string
  /** Every complete week in the window, empty ones included. */
  weeks: TrendWeek[]
  median_seconds: number | null
  signals: Signal[]
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
   * What the team is doing right now. Polled on a timer, so it is deliberately
   * separate from `teamDays`: re-running the week's totals every half minute
   * would be the heaviest query on the server answering with numbers that did
   * not change.
   */
  teamLive: () => request<LiveTeamResponse>('/team/live'),

  /**
   * The team's month, a cell per person per recorded day.
   *
   * `month` is `YYYY-MM`. The server refuses anything else rather than
   * guessing - a caller who sent a date would otherwise get a plausible answer
   * to a question they did not ask.
   */
  teamHeatmap: (month: string) => request<HeatmapResponse>(`/team/heatmap?month=${month}`),

  /**
   * What the server thinks is worth a look. Asked once with the dashboard
   * rather than polled: it is about whole weeks, and whole weeks do not change
   * while somebody has the page open.
   */
  teamSignals: () => request<SignalsResponse>('/team/signals'),

  /**
   * A year of the production calendar: only the dates that differ from the
   * weekday they fall on. Readable by anyone signed in - which days of the
   * year are worked is not a secret from the people working them.
   */
  calendar: (year: number) => request<CalendarYear>(`/calendar?year=${year}`),

  /**
   * Replaces a year of the calendar. Administrators only, and a replacement
   * rather than a merge: a corrected calendar is the document that is right.
   */
  putCalendar: (year: number, days: CalendarDay[]) =>
    request<CalendarYear>(`/calendar?year=${year}`, { method: 'PUT', body: JSON.stringify({ days }) }),

  /** Sets the installation's full day, in hours. Administrators only. */
  putStandardHours: (standard_hours: number) =>
    request<{ standard_hours: number }>('/calendar/standard-hours', {
      method: 'PUT',
      body: JSON.stringify({ standard_hours }),
    }),

  /** Sets one person's share of a full day. Administrators only. */
  putWorkRate: (id: string, work_rate: number) =>
    request<{ work_rate: number }>(`/users/${id}/work-rate`, { method: 'PUT', body: JSON.stringify({ work_rate }) }),

  /** One person's twelve-week shape, and the signals about them. */
  userTrend: (id: string) => request<TrendResponse>(`/users/${id}/trend`),

  /**
   * One person's days, for a manager who may see them.
   *
   * Answers exactly what `myDays` does - the drill-down is the personal screen
   * pointed at someone else, so the two share a renderer.
   */
  userDays: (id: string, from: string, to: string) => request<DaysResponse>(`/users/${id}/days?from=${from}&to=${to}`),
}
