import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useParams } from 'react-router'
import { ArrowLeft, Circle, CircleDot, TriangleAlert } from 'lucide-react'
import { api, type LiveMember, type Member, type TeamResponse } from '@/lib/api'
import { duration, isoDate, shiftWeeks, since, startOfWeek, weekDates } from '@/lib/day'
import { statusTone, useLiveTeam, type LiveFeed } from '@/lib/live'
import { Panel } from '@/components/ui/panel'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import { Track } from '@/components/ui/track'
import { PeriodPicker } from '@/components/PeriodPicker'
import { WeekView } from '@/pages/MyDay'
import { Alerts } from '@/components/Alerts'
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
      <header className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold">{t('team.title')}</h1>
          <p className="mt-1 font-mono text-xs text-faint tabular">
            {from} — {to}
          </p>
        </div>
        <PeriodPicker
          previousLabel={t('myDay.previousWeek')}
          nextLabel={t('myDay.nextWeek')}
          onPrevious={() => goto(-1)}
          onNext={() => goto(1)}
          onNow={() => setMonday(startOfWeek(new Date()))}
        >
          {t('myDay.thisWeek')}
        </PeriodPicker>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {/* What the server noticed on its own, first. Above the signals, and
          that order is deliberate: a slide over three weeks will still be a
          slide tomorrow, while a machine that has said nothing for thirty
          hours gets less actionable the longer it waits. The thing that
          decays goes on top. Like the signals, outside the week's loading
          state - neither is about the week being paged through. */}
      <Alerts />

      {/* Above the table and outside the week's loading state: the signals are
          about whole weeks and do not change when the manager pages back
          through them, so they must not blink on every arrow press. */}
      <Signals />

      {answer && (
        <>
          <TeamTotals answer={answer} live={live} />
          <MemberTable members={answer.members} live={live} />
        </>
      )}
    </div>
  )
}

/** The week across everyone, so the table has something to be measured against. */
function TeamTotals({ answer, live }: { answer: TeamResponse; live: LiveFeed }) {
  const { t } = useTranslation()
  const members = answer.members
  const worked = members.reduce((sum, member) => sum + member.worked_seconds, 0)
  // The team's norm is the sum of the people's, not a full week times the head
  // count: half-timers and people on leave each owe their own figure, and a
  // team total that ignored them would be a number nobody is measured by.
  //
  // And only the people this server actually measures. An account with no
  // agent installed - the administrator's own, somebody who has not set kasl
  // up yet - owes hours nothing will ever report, and adding them made the
  // pair read "219h of 448h" for a team that had in fact worked most of what
  // it owed. Seen on the demo, where two of twelve have no agent; the arithmetic
  // was right and the sentence it formed was false.
  const measured = members.filter((member) => member.agents > 0)
  const norm = measured.reduce((sum, member) => sum + member.norm_seconds, 0)
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
    <Panel className="p-4 sm:p-5">
      <StatRow>
        <StatTile label={t('team.workedTotal')} value={duration(worked)} tone="accent" />
        {/* The figure the hours are measured against, as its own tile rather
            than as the accented one's delta: a delta says which way something
            moved, and a norm is not a movement. The two side by side are the
            pair the server answers (ADR 0017). */}
        {norm > 0 && <StatTile label={t('team.normTotal')} value={duration(norm)} />}
        <StatTile label={t('team.people')} value={String(members.length)} />
        {live.loaded && <StatTile label={t('team.workingNow')} value={String(working)} />}
        <StatTile label={t('team.dayOpen')} value={String(open)} />
        {silent > 0 && (
          <StatTile
            label={t('team.silent')}
            // The warning tone is not carrying this on its own. This product's
            // accent is gold, and gold against the warning colour measures ΔE
            // 3.7 - one colour, to a reader glancing at a row where the
            // accented total sits three tiles away. The mark says which figure
            // is the problem without depending on telling two golds apart.
            value={
              <span className="inline-flex items-center gap-1.5">
                <TriangleAlert className="size-4 shrink-0" />
                {silent}
              </span>
            }
            tone="warn"
          />
        )}
      </StatRow>
    </Panel>
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

  // The scale the bars share. Before there was a norm this was the longest
  // week in view - people against each other, because nothing else existed to
  // measure them by. Now a person's own norm is the right edge where they have
  // one: a bar that fills is a week worked in full, which is a fact about that
  // person rather than about whoever happened to work longest.
  //
  // The longest week still sets the floor, so a team where everybody is over
  // their norm does not draw seven identical full bars.
  const longest = Math.max(...members.map((member) => Math.max(member.worked_seconds, member.norm_seconds)), 1)

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
  // Nothing at all - not even a day off. Somebody who was away all week has
  // told us something, and "no data recorded" would be the wrong sentence for
  // it (the row says how many days away instead).
  const nothing = member.days_recorded === 0 && member.days_away === 0

  const total = <div className="font-mono text-sm tabular">{nothing ? '—' : duration(member.worked_seconds)}</div>
  const lastDay = member.last_day && <div className="font-mono text-[11px] text-faint tabular">{member.last_day}</div>

  return (
    <Link
      to={`/team/${member.id}`}
      className="flex flex-col gap-2 px-4 py-3.5 transition-colors hover:bg-soft sm:flex-row sm:items-center sm:gap-4 sm:px-5"
    >
      {/* The name and the week's total share the phone's top line; the bar
          takes the width below them. The desktop's three columns would leave
          the bar narrower than the name beside it, and the bar is the only
          thing on the row that compares one person to another. */}
      <div className="flex items-baseline justify-between gap-3 sm:contents">
        <div className="min-w-0 sm:w-52 sm:shrink-0">
          <div className="truncate text-sm font-medium">{member.display_name}</div>
          <div className="truncate text-[11px] text-faint">{member.department ?? t('team.noDepartment')}</div>
        </div>
        <div className="shrink-0 text-right sm:hidden">
          {total}
          {lastDay}
        </div>
      </div>

      <div className="min-w-0 flex-1">
        {nothing ? (
          // Not an empty bar: a bar of zero length reads as "worked nothing",
          // which is a claim. The words say which of the two this is.
          <p className="text-sm text-faint">{t('team.noData')}</p>
        ) : (
          // One segment on the scale the table shares, so the bars can be read
          // against each other and against the notch that marks this person's
          // own norm. `minWidth={0}` because these widths are being read by
          // eye - a widened bar is no longer to scale, and a reader measuring
          // it would be measuring the floor.
          <div className="relative">
            <Track
              segments={[{ key: 'worked', start: 0, end: member.worked_seconds }]}
              from={0}
              to={longest}
              minWidth={0}
              label={
                member.norm_seconds > 0
                  ? t('team.normBar', {
                      name: member.display_name,
                      hours: duration(member.worked_seconds),
                      norm: duration(member.norm_seconds),
                    })
                  : t('team.workedBar', { name: member.display_name, hours: duration(member.worked_seconds) })
              }
            />
            {member.norm_seconds > 0 && member.norm_seconds < longest && (
              // Where this person's week was due to end. A hairline rather
              // than a second bar: the row is about the hours, and the norm is
              // the mark they are read against.
              //
              // `aria-hidden` because the bar's own label already names both
              // figures - a screen reader meeting a second, wordless element
              // here would hear an interruption, not a fact.
              <span
                aria-hidden
                className="pointer-events-none absolute inset-y-0 w-px bg-dim"
                style={{ left: `${(member.norm_seconds / longest) * 100}%` }}
              />
            )}
          </div>
        )}
        <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-faint tabular">
          {!nothing && (
            <span>
              {member.days_recorded} {t('team.days')} · {duration(member.paused_seconds)} {t('team.pausedShort')}
            </span>
          )}
          {/* Why a short week is short. Without it a fortnight of leave reads
              as somebody who stopped working (ADR 0017). */}
          {member.days_away > 0 && <span>{t('team.daysAway', { count: member.days_away })}</span>}
          {member.work_rate !== 1 && <span>{t('team.partTime', { rate: member.work_rate })}</span>}
          <Status member={member} live={live} />
        </div>
      </div>

      <div className="hidden shrink-0 text-right sm:block">
        {total}
        {lastDay && <div className="mt-0.5">{lastDay}</div>}
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
