import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useParams } from 'react-router'
import { ArrowLeft, ChevronLeft, ChevronRight, Circle, CircleDot, TriangleAlert } from 'lucide-react'
import { api, type LiveMember, type Member, type TeamResponse } from '@/lib/api'
import { duration, isoDate, shiftWeeks, since, startOfWeek, weekDates } from '@/lib/day'
import { statusTone, useLiveTeam, type LiveFeed } from '@/lib/live'
import { Panel } from '@/components/ui/panel'
import { Button } from '@/components/ui/button'
import { WeekView } from '@/pages/MyDay'
import { Signals } from '@/components/Signals'
import { Trend } from '@/components/Trend'

/**
 * The manager's dashboard: the team over a week, a row per person.
 *
 * Everyone the reader may see is listed, including people with nothing
 * recorded. An employee whose agent has never reported is exactly who a
 * manager needs to notice, and a table that quietly dropped them would hide
 * the case it exists for.
 *
 * "Working now" is the agent's own claim, polled from `/team/live` on the
 * cadence the server names. It is kept apart from the week's hours on purpose:
 * an agent that stopped sending is shown as offline rather than frozen on its
 * last claim, and a person whose kasl is too old to send a pulse reads as
 * "unknown" rather than as someone who stopped working (ADR 0014).
 */
export function Dashboard() {
  const { t } = useTranslation()
  const [monday, setMonday] = useState(() => startOfWeek(new Date()))
  // Keyed by the range it answers, so a late reply for the week just left
  // cannot land as if it were this one's.
  const [loaded, setLoaded] = useState<{ range: string; answer: TeamResponse | null } | null>(null)

  const dates = useMemo(() => weekDates(monday), [monday])
  const from = dates[0]
  const to = dates[6]
  const range = `${from}:${to}`

  useEffect(() => {
    let cancelled = false
    api
      .teamDays(from, to)
      .then((value) => {
        if (!cancelled) setLoaded({ range: `${from}:${to}`, answer: value })
      })
      .catch(() => {
        if (!cancelled) setLoaded({ range: `${from}:${to}`, answer: null })
      })
    return () => {
      cancelled = true
    }
  }, [from, to])

  const current = loaded?.range === range ? loaded : null
  const answer = current?.answer ?? null
  const failed = current !== null && current.answer === null

  // The pulse is about now, so it is asked for regardless of which week the
  // table is showing - a manager paging back through August still wants to see
  // who is at work today.
  const live = useLiveTeam()

  const goto = useCallback((weeks: number) => setMonday((current) => shiftWeeks(current, weeks)), [])

  return (
    <div className="mx-auto max-w-5xl space-y-5">
      <header className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-lg font-semibold">{t('team.title')}</h1>
          <p className="mt-1 font-mono text-xs text-faint tabular">
            {from} — {to}
          </p>
        </div>
        <div className="flex items-center gap-1.5">
          <Button variant="icon" size="icon-md" aria-label={t('myDay.previousWeek')} onClick={() => goto(-1)}>
            <ChevronLeft />
          </Button>
          <Button size="sm" onClick={() => setMonday(startOfWeek(new Date()))}>
            {t('myDay.thisWeek')}
          </Button>
          <Button variant="icon" size="icon-md" aria-label={t('myDay.nextWeek')} onClick={() => goto(1)}>
            <ChevronRight />
          </Button>
        </div>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {/* Above the table and outside the week's loading state: the signals are
          about whole weeks and do not change when the manager pages back
          through them, so they must not blink on every arrow press. */}
      <Signals />

      {answer && (
        <>
          <TeamTotals members={answer.members} live={live} />
          <MemberTable members={answer.members} live={live} />
        </>
      )}
    </div>
  )
}

/** The week across everyone, so the table has something to be measured against. */
function TeamTotals({ members, live }: { members: Member[]; live: LiveFeed }) {
  const { t } = useTranslation()
  const worked = members.reduce((sum, member) => sum + member.worked_seconds, 0)
  const open = members.filter((member) => member.day_open).length
  // People a manager should look at: no agent at all, or one that has never
  // delivered anything. Counted rather than buried, because this is the
  // question the dashboard is for.
  const silent = members.filter((member) => member.agents === 0 || member.last_seen_at === null).length
  // At the keyboard right now, by their own agent's account. Shown only once a
  // pulse has actually been answered: a hard "0 working" drawn before the first
  // poll lands would be a claim, and a false one.
  const working = members.filter((member) => live.byUser.get(member.id)?.status === 'working').length

  return (
    <Panel className="flex flex-wrap items-baseline gap-x-8 gap-y-3 p-5">
      <Figure label={t('team.workedTotal')} value={duration(worked)} accent />
      <Figure label={t('team.people')} value={String(members.length)} />
      {live.loaded && <Figure label={t('team.workingNow')} value={String(working)} />}
      <Figure label={t('team.dayOpen')} value={String(open)} />
      {silent > 0 && <Figure label={t('team.silent')} value={String(silent)} warn />}
    </Panel>
  )
}

function Figure({ label, value, accent = false, warn = false }: { label: string; value: string; accent?: boolean; warn?: boolean }) {
  return (
    <div>
      <div className="text-xs font-medium text-dim">{label}</div>
      <div className={`mt-1 font-mono text-lg tabular ${accent ? 'text-accent-2' : ''}${warn ? 'text-warn' : ''}`}>{value}</div>
    </div>
  )
}

function MemberTable({ members, live }: { members: Member[]; live: LiveFeed }) {
  const { t } = useTranslation()

  if (members.length === 0) {
    return (
      <Panel className="p-5">
        <p className="text-sm text-dim">{t('team.nobody')}</p>
      </Panel>
    )
  }

  // The longest week in view sets the bar scale, so the bars compare people
  // with each other rather than against a number nobody chose.
  const longest = Math.max(...members.map((member) => member.worked_seconds), 1)

  return (
    <Panel className="divide-y divide-line">
      {members.map((member) => (
        <MemberRow key={member.id} member={member} longest={longest} live={live.byUser.get(member.id)} />
      ))}
    </Panel>
  )
}

function MemberRow({ member, longest, live }: { member: Member; longest: number; live: LiveMember | undefined }) {
  const { t } = useTranslation()
  const nothing = member.days_recorded === 0

  return (
    <Link to={`/team/${member.id}`} className="flex items-center gap-4 px-5 py-3.5 transition-colors hover:bg-soft">
      <div className="min-w-0 w-52 shrink-0">
        <div className="truncate text-sm font-medium">{member.display_name}</div>
        <div className="truncate text-[11px] text-faint">{member.department ?? t('team.noDepartment')}</div>
      </div>

      <div className="min-w-0 flex-1">
        {nothing ? (
          // Not an empty bar: a bar of zero length reads as "worked nothing",
          // which is a claim. The words say which of the two this is.
          <p className="text-sm text-faint">{t('team.noData')}</p>
        ) : (
          <div className="h-2.5 overflow-hidden rounded-full bg-softer">
            <div className="h-full rounded-full bg-accent" style={{ width: `${(member.worked_seconds / longest) * 100}%` }} />
          </div>
        )}
        <div className="mt-1.5 flex items-center gap-3 font-mono text-[11px] text-faint tabular">
          {!nothing && (
            <span>
              {member.days_recorded} {t('team.days')} · {duration(member.paused_seconds)} {t('team.pausedShort')}
            </span>
          )}
          <Status member={member} live={live} />
        </div>
      </div>

      <div className="shrink-0 text-right">
        <div className="font-mono text-sm tabular">{nothing ? '—' : duration(member.worked_seconds)}</div>
        {member.last_day && <div className="mt-0.5 font-mono text-[11px] text-faint tabular">{member.last_day}</div>}
      </div>
    </Link>
  )
}

/** The colour roles `statusTone` names, as literal classes Tailwind can find. */
const TONE_CLASS = {
  good: 'text-good',
  warn: 'text-warn',
  dim: 'text-dim',
  faint: 'text-faint',
} as const

/**
 * What the server knows about this person right now.
 *
 * The pulse wins when there is one: it is the agent's own claim about this
 * minute, where everything else on the row is about days already filed. When
 * there is none - no agent, or a kasl too old to send one - the row falls back
 * to what it said before this milestone, which is still true.
 *
 * Icons carry the meaning alongside the colour, as the mockup requires.
 */
function Status({ member, live }: { member: Member; live: LiveMember | undefined }) {
  const { t } = useTranslation()

  if (member.agents === 0) {
    return (
      <span className="inline-flex items-center gap-1 text-warn">
        <TriangleAlert className="size-3" />
        {t('team.noAgent')}
      </span>
    )
  }

  // `unknown` is not shown as a live status: it means no pulse ever arrived,
  // and the row below already has better words for that case - "never
  // reported", or the date of the last data.
  if (live && live.status !== 'unknown') {
    const { tone, live: atWork } = statusTone(live.status)
    return (
      // Spelled out rather than interpolated: Tailwind scans source text for
      // class names, and `text-${tone}` is a class it never sees and never
      // emits.
      <span className={`inline-flex items-center gap-1 ${TONE_CLASS[tone]}`}>
        {atWork ? <CircleDot className="size-3" /> : <Circle className="size-3" />}
        {t(`team.live.${live.status}`)}
        {/* An offline row says when it went quiet: "offline" alone leaves a
            manager wondering whether it happened a minute or a week ago. */}
        {live.status === 'offline' && member.last_seen_at && ` · ${t('team.lastSeen', { ago: sinceText(member.last_seen_at, t) })}`}
      </span>
    )
  }

  if (member.day_open) {
    return (
      <span className="inline-flex items-center gap-1 text-good">
        <CircleDot className="size-3" />
        {t('team.open')}
        {member.last_seen_at && ` · ${t('team.lastSeen', { ago: sinceText(member.last_seen_at, t) })}`}
      </span>
    )
  }

  if (member.last_seen_at === null) {
    return (
      <span className="inline-flex items-center gap-1 text-warn">
        <TriangleAlert className="size-3" />
        {t('team.neverReported')}
      </span>
    )
  }

  return <span>{t('team.lastSeen', { ago: sinceText(member.last_seen_at, t) })}</span>
}

/** "12 min", "3 h", "5 d" - the unit `since` chose, in the reader's language. */
function sinceText(timestamp: string, t: (key: string, options?: Record<string, unknown>) => string): string {
  const [unit, count] = since(timestamp)
  return t(`team.${unit}Ago`, { count })
}

/**
 * One person's week, opened from the dashboard.
 *
 * Renders through the same component as the employee's own page: the server
 * answers the identical shape for `/me/days` and `/users/{id}/days`, so the
 * drill-down is that screen pointed at someone else.
 */
export function PersonWeek() {
  const { t } = useTranslation()
  const { id } = useParams<{ id: string }>()
  const [name, setName] = useState<string | null>(null)

  // The name is not on the days endpoint - it answers days, not people. Rather
  // than add it there for one label, the dashboard's own answer is asked for
  // the current week, which is already cached in most arrivals here.
  useEffect(() => {
    if (!id) return
    let cancelled = false
    const today = isoDate(new Date())
    api
      .teamDays(today, today)
      .then((team) => {
        if (cancelled) return
        setName(team.members.find((member) => member.id === id)?.display_name ?? null)
      })
      .catch(() => {
        // A missing label is not worth an error message on a page whose data
        // loads independently.
      })
    return () => {
      cancelled = true
    }
  }, [id])

  const load = useCallback((from: string, to: string) => api.userDays(id!, from, to), [id])

  if (!id) return null

  return (
    <div className="space-y-4">
      <Link to="/team" className="inline-flex items-center gap-1.5 text-sm text-dim transition-colors hover:text-text">
        <ArrowLeft className="size-3.5" />
        {t('team.backToTeam')}
      </Link>
      {/* The chart comes first: this page is usually arrived at from a signal,
          and the twelve weeks are what the signal was about. */}
      <Trend userId={id} />
      <WeekView title={name ?? t('team.person')} load={load} />
    </div>
  )
}
