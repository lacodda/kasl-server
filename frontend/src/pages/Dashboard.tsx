import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useParams } from 'react-router'
import { ArrowLeft, ChevronLeft, ChevronRight, CircleDot, TriangleAlert } from 'lucide-react'
import { api, type Member, type TeamResponse } from '@/lib/api'
import { duration, isoDate, shiftWeeks, since, startOfWeek, weekDates } from '@/lib/day'
import { Panel } from '@/components/ui/Panel'
import { Button } from '@/components/ui/Button'
import { WeekView } from '@/pages/MyDay'

/**
 * The manager's dashboard: the team over a week, a row per person.
 *
 * Everyone the reader may see is listed, including people with nothing
 * recorded. An employee whose agent has never reported is exactly who a
 * manager needs to notice, and a table that quietly dropped them would hide
 * the case it exists for.
 *
 * "Working now" is deliberately not claimed. The server knows whether a day is
 * open and when it last heard from an agent; whether someone is at the keyboard
 * needs heartbeats, which is a later milestone. The row says what is known.
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
          <Button variant="icon" size="iconMd" aria-label={t('myDay.previousWeek')} onClick={() => goto(-1)}>
            <ChevronLeft />
          </Button>
          <Button size="sm" onClick={() => setMonday(startOfWeek(new Date()))}>
            {t('myDay.thisWeek')}
          </Button>
          <Button variant="icon" size="iconMd" aria-label={t('myDay.nextWeek')} onClick={() => goto(1)}>
            <ChevronRight />
          </Button>
        </div>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {answer && (
        <>
          <TeamTotals members={answer.members} />
          <MemberTable members={answer.members} />
        </>
      )}
    </div>
  )
}

/** The week across everyone, so the table has something to be measured against. */
function TeamTotals({ members }: { members: Member[] }) {
  const { t } = useTranslation()
  const worked = members.reduce((sum, member) => sum + member.worked_seconds, 0)
  const working = members.filter((member) => member.day_open).length
  // People a manager should look at: no agent at all, or one that has never
  // delivered anything. Counted rather than buried, because this is the
  // question the dashboard is for.
  const silent = members.filter((member) => member.agents === 0 || member.last_seen_at === null).length

  return (
    <Panel className="flex flex-wrap items-baseline gap-x-8 gap-y-3 p-5">
      <Figure label={t('team.workedTotal')} value={duration(worked)} accent />
      <Figure label={t('team.people')} value={String(members.length)} />
      <Figure label={t('team.dayOpen')} value={String(working)} />
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

function MemberTable({ members }: { members: Member[] }) {
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
        <MemberRow key={member.id} member={member} longest={longest} />
      ))}
    </Panel>
  )
}

function MemberRow({ member, longest }: { member: Member; longest: number }) {
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
          <Status member={member} />
        </div>
      </div>

      <div className="shrink-0 text-right">
        <div className="font-mono text-sm tabular">{nothing ? '—' : duration(member.worked_seconds)}</div>
        {member.last_day && <div className="mt-0.5 font-mono text-[11px] text-faint tabular">{member.last_day}</div>}
      </div>
    </Link>
  )
}

/**
 * What the server actually knows about this person right now.
 *
 * Never "working": an open day plus a recent delivery is as far as the evidence
 * goes, and the label says exactly that. Icons carry the meaning alongside the
 * colour, as the mockup requires.
 */
function Status({ member }: { member: Member }) {
  const { t } = useTranslation()

  if (member.agents === 0) {
    return (
      <span className="inline-flex items-center gap-1 text-warn">
        <TriangleAlert className="size-3" />
        {t('team.noAgent')}
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
      <WeekView title={name ?? t('team.person')} load={load} />
    </div>
  )
}
