import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useParams, useSearchParams } from 'react-router'
import { ArrowLeft, Circle, CircleDot, TriangleAlert, UserX } from 'lucide-react'
import { api, type LiveMember, type Member, type TeamResponse } from '@/lib/api'
import {
  compareMembers,
  departmentTotals,
  measured,
  points,
  SORT_KEYS,
  sortCompared,
  totals,
  type Compared,
  type DepartmentTotals,
  type SortKey,
} from '@/lib/compare'
import { duration, isoDate, since } from '@/lib/day'
import { statusTone, useLiveTeam, type LiveFeed } from '@/lib/live'
import { percent, periodFromParams, periodParams, shareChange, shiftPeriod, type Period, type PeriodUnit } from '@/lib/period'
import { usePeriod } from '@/lib/use-period'
import { Panel } from '@/components/ui/panel'
import { Segment, SegmentedControl } from '@/components/ui/segmented-control'
import { StatRow, StatTile } from '@/components/ui/stat-tile'
import { Track } from '@/components/ui/track'
import { ExportMenu } from '@/components/ExportMenu'
import { PeriodControls, PeriodRange } from '@/components/PeriodControls'
import { WeekView } from '@/pages/MyDay'
import { Alerts } from '@/components/Alerts'
import { Approvals } from '@/components/Approvals'
import { Signals } from '@/components/Signals'
import { Trend } from '@/components/Trend'

/**
 * The manager's dashboard: the team over a period, a row per person.
 *
 * Everyone the reader may see is listed, including people with nothing
 * recorded. An employee whose agent has never reported is exactly who a
 * manager needs to notice, and a table that quietly dropped them would hide
 * the case it exists for.
 *
 * The period is a day, a week or a month, kept in the URL (ADR 0023). Each
 * person is shown against their own norm and against themselves in the period
 * before - never ranked by hours against each other (ADR 0016).
 *
 * "Working now" is the agent's own claim, polled from `/team/live` on the
 * cadence the server names. It is kept apart from the period's hours on
 * purpose: an agent that stopped sending is shown as offline rather than
 * frozen on its last claim, and a person whose kasl is too old to send a pulse
 * reads as "unknown" rather than as someone who stopped working (ADR 0014).
 */
export function Dashboard() {
  const { t } = useTranslation()
  const [period, setPeriod] = usePeriod()
  const [sort, setSort] = useState<SortKey>('name')
  // Keyed by the range it answers, so a late reply for the period just left
  // cannot land as if it were this one's. The period before is a second
  // answer of the same endpoint: the comparison is arithmetic on two of them.
  const [loaded, setLoaded] = useState<{ range: string; answer: TeamResponse | null; before: TeamResponse | null } | null>(null)

  // Whole periods. What a period still running is measured against is the
  // norm that has come due, which the server answers beside the whole norm
  // (ADR 0023); a period not yet begun has none due, so nobody has a share of
  // it and nothing is compared.
  const { from, to } = period
  const { from: beforeFrom, to: beforeTo } = shiftPeriod(period, -1)
  const range = `${from}:${to}`

  useEffect(() => {
    let cancelled = false
    const key = `${from}:${to}`
    Promise.all([
      api.teamDays(from, to),
      // A failed comparison costs the comparison, not the screen.
      api.teamDays(beforeFrom, beforeTo).catch(() => null),
    ])
      .then(([answer, before]) => {
        if (!cancelled) setLoaded({ range: key, answer, before })
      })
      .catch(() => {
        if (!cancelled) setLoaded({ range: key, answer: null, before: null })
      })
    return () => {
      cancelled = true
    }
  }, [from, to, beforeFrom, beforeTo])

  const current = loaded?.range === range ? loaded : null
  const answer = current?.answer ?? null
  const before = current?.before ?? null
  const failed = current !== null && current.answer === null

  // The pulse is about now, so it is asked for regardless of which period the
  // table is showing - a manager paging back through August still wants to
  // see who is at work today.
  const live = useLiveTeam()

  return (
    <div className="mx-auto max-w-5xl space-y-5">
      <header className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
        <div className="min-w-0">
          <h1 className="text-lg font-semibold">{t('team.title')}</h1>
          <PeriodRange period={period} />
        </div>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
          <PeriodControls period={period} onChange={setPeriod} />
          <ExportMenu subject="team" period={period} />
        </div>
      </header>

      {failed && <p className="text-sm text-bad">{t('common.error')}</p>}
      {current === null && <p className="text-sm text-dim">{t('common.loading')}</p>}

      {/* What the server noticed on its own, first. Above the signals, and
          that order is deliberate: a slide over three weeks will still be a
          slide tomorrow, while a machine that has said nothing for thirty
          hours gets less actionable the longer it waits. The thing that
          decays goes on top. Like the signals, outside the period's loading
          state - neither is about the period being paged through. */}
      <Alerts />

      {/* What people asked the reader to approve. Under the alerts - a fire
          goes first - and above the signals, which will still be true next
          week; somebody is waiting on this one. */}
      <Approvals />

      {/* Above the table and outside the period's loading state: the signals
          are about whole weeks and do not change when the manager pages back
          through them, so they must not blink on every arrow press. */}
      <Signals />

      {answer && (
        <>
          <TeamTotals answer={answer} before={before} unit={period.unit} live={live} />
          <Departments members={answer.members} before={before?.members ?? null} unit={period.unit} />
          <MemberTable
            rows={sortCompared(compareMembers(answer.members, before?.members ?? null), sort)}
            sort={sort}
            onSort={setSort}
            period={period}
            live={live}
          />
        </>
      )}
    </div>
  )
}

/** The period across everyone, so the table has something to be measured against. */
function TeamTotals({
  answer,
  before,
  unit,
  live,
}: {
  answer: TeamResponse
  before: TeamResponse | null
  unit: PeriodUnit
  live: LiveFeed
}) {
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
  const team = totals(members)
  const whole = members.filter(measured).reduce((sum, member) => sum + member.norm_seconds, 0)
  const change = shareChange(team.share, before ? totals(before.members).share : null)
  const open = members.filter((member) => member.day_open).length
  // People a manager should look at: no agent at all, or one that has never
  // delivered anything. Counted rather than buried, because this is the
  // question the dashboard is for. Only the people still here: somebody who
  // left is listed for the hours they worked, not as a silence to chase.
  const silent = members.filter((member) => member.active && (member.agents === 0 || member.last_seen_at === null)).length
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
        {/* What has come due, which for a period over is its whole norm and
            for one running is the part of it behind today - the same figure
            the share divides by, so the three tiles agree. */}
        {team.due_seconds > 0 && (
          <StatTile
            label={t('team.normTotal')}
            value={duration(team.due_seconds)}
            delta={whole > team.due_seconds ? t(`myDay.wholeNorm.${unit}`, { hours: duration(whole) }) : undefined}
          />
        )}
        {/* The share is where the period before comes in: September against
            August as shares, because their hours would compare calendars. */}
        {team.share !== null && (
          <StatTile
            label={t('team.ofNormTotal')}
            value={percent(team.share)}
            delta={change !== null ? `${points(change)} ${t(`period.versus.${unit}`)}` : undefined}
          />
        )}
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

/**
 * The period by department, for a reader who sees more than one.
 *
 * A manager of one department already has its figures in the totals above; a
 * second table saying them again would be noise. An administrator with three
 * departments gets the comparison the team totals cannot give - each one
 * against its own norm and against itself in the period before.
 */
function Departments({ members, before, unit }: { members: Member[]; before: Member[] | null; unit: PeriodUnit }) {
  const { t } = useTranslation()
  const groups = useMemo(() => departmentTotals(members, before), [members, before])
  if (groups.length < 2) return null

  return (
    <section aria-labelledby="departments-heading" className="space-y-2">
      <h2 id="departments-heading" className="text-sm font-medium text-dim">
        {t('team.departments')}
      </h2>
      <Panel className="divide-y divide-line">
        {groups.map((group) => (
          <DepartmentRow key={group.name ?? ''} group={group} unit={unit} />
        ))}
      </Panel>
    </section>
  )
}

function DepartmentRow({ group, unit }: { group: DepartmentTotals; unit: PeriodUnit }) {
  const { t } = useTranslation()
  return (
    <div className="flex items-baseline justify-between gap-3 px-4 py-2.5 sm:px-5">
      <div className="min-w-0">
        <div className="truncate text-sm">{group.name ?? t('team.noDepartment')}</div>
        <div className="font-mono text-[11px] text-faint tabular">{t('team.peopleCount', { count: group.people })}</div>
      </div>
      <div className="shrink-0 text-right font-mono text-sm tabular">
        <div>
          {duration(group.worked_seconds)}
          {group.share !== null && <span className="ml-2 text-dim">{t('team.ofNormShort', { share: percent(group.share) })}</span>}
        </div>
        {group.change !== null && (
          <div className="text-[11px] text-faint">
            {points(group.change)} {t(`period.versus.${unit}`)}
          </div>
        )}
      </div>
    </div>
  )
}

function MemberTable({
  rows,
  sort,
  onSort,
  period,
  live,
}: {
  rows: Compared[]
  sort: SortKey
  onSort: (key: SortKey) => void
  period: Period
  live: LiveFeed
}) {
  const { t } = useTranslation()

  if (rows.length === 0) {
    return (
      <Panel className="p-5">
        <p className="text-sm text-dim">{t('team.nobody')}</p>
      </Panel>
    )
  }

  // The scale the bars share. Before there was a norm this was the longest
  // period in view - people against each other, because nothing else existed
  // to measure them by. Now a person's own norm is the right edge where they
  // have one: a bar that fills is a period worked in full, which is a fact
  // about that person rather than about whoever happened to work longest.
  //
  // The longest period still sets the floor, so a team where everybody is over
  // their norm does not draw seven identical full bars.
  const longest = Math.max(...rows.map(({ member }) => Math.max(member.worked_seconds, member.due_seconds)), 1)

  return (
    <section className="space-y-2">
      {/* The orders the table offers, and none by hours: that order is a
          scoreboard, a full-timer always above somebody on half time
          (ADR 0016). Share and change put who to look at first. */}
      <div className="flex items-center justify-end gap-2">
        <span id="sort-label" className="text-xs text-faint">
          {t('team.sortLabel')}
        </span>
        <SegmentedControl aria-labelledby="sort-label" value={sort} onValueChange={(value) => onSort(value as SortKey)}>
          {SORT_KEYS.map((key) => (
            <Segment key={key} value={key}>
              {t(`team.sort.${key}`)}
            </Segment>
          ))}
        </SegmentedControl>
      </div>
      <Panel className="divide-y divide-line">
        {rows.map((row) => (
          <MemberRow key={row.member.id} row={row} longest={longest} period={period} live={live.byUser.get(row.member.id)} />
        ))}
      </Panel>
    </section>
  )
}

function MemberRow({ row, longest, period, live }: { row: Compared; longest: number; period: Period; live: LiveMember | undefined }) {
  const { t } = useTranslation()
  const { member, share, change } = row
  // Nothing at all - not even a day off. Somebody who was away all week has
  // told us something, and "no data recorded" would be the wrong sentence for
  // it (the row says how many days away instead).
  const nothing = member.days_recorded === 0 && member.days_away === 0

  const total = <div className="font-mono text-sm tabular">{nothing ? '—' : duration(member.worked_seconds)}</div>
  const lastDay = member.last_day && <div className="font-mono text-[11px] text-faint tabular">{member.last_day}</div>
  // Where this person stands against their own norm, and against themselves
  // in the period before - the comparison this table makes, rather than one
  // person against another (ADR 0016).
  const standing = share !== null && (
    <div className="font-mono text-[11px] text-dim tabular">
      {t('team.ofNormShort', { share: percent(share) })}
      {change !== null && (
        <span className="ml-1.5 text-faint">
          {points(change)}
          <span className="sr-only"> {t(`period.versus.${period.unit}`)}</span>
        </span>
      )}
    </div>
  )

  return (
    <Link
      // The same period on their page, so a month opened here is a month
      // there, and the way back lands on it again.
      to={`/team/${member.id}?${new URLSearchParams(periodParams(period))}`}
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
          {standing}
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
                member.due_seconds > 0
                  ? t('team.normBar', {
                      name: member.display_name,
                      hours: duration(member.worked_seconds),
                      norm: duration(member.due_seconds),
                    })
                  : t('team.workedBar', { name: member.display_name, hours: duration(member.worked_seconds) })
              }
            />
            {member.due_seconds > 0 && member.due_seconds < longest && (
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
                style={{ left: `${(member.due_seconds / longest) * 100}%` }}
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
        {standing}
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

  // Listed for the hours they worked in this period, not as somebody to chase:
  // the account is closed, and "no agent" or "offline" would ask a question
  // nobody needs to answer.
  if (!member.active) {
    return (
      <span className="inline-flex items-center gap-1">
        <UserX className="size-3" />
        {t('team.left')}
      </span>
    )
  }

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
  const [params] = useSearchParams()
  // The period on the way in. The arrows on this page move it; the way back
  // to the team lands on wherever they left it.
  const [arrival] = useState(() => periodFromParams(params, isoDate(new Date())))
  const [period] = usePeriod()

  // The name is not on the days endpoint - it answers days, not people. Rather
  // than add it there for one label, the dashboard's own answer is asked for
  // the period this page was opened on: the row that was clicked was in it,
  // even for somebody who has left since.
  useEffect(() => {
    if (!id) return
    let cancelled = false
    api
      .teamDays(arrival.from, arrival.to)
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
  }, [id, arrival.from, arrival.to])

  const load = useCallback((from: string, to: string) => api.userDays(id!, from, to), [id])

  if (!id) return null

  return (
    <div className="space-y-4">
      <Link
        to={`/team?${new URLSearchParams(periodParams(period))}`}
        className="inline-flex items-center gap-1.5 text-sm text-dim transition-colors hover:text-text"
      >
        <ArrowLeft className="size-3.5" />
        {t('team.backToTeam')}
      </Link>
      {/* The chart comes first: this page is usually arrived at from a signal,
          and the twelve weeks are what the signal was about. */}
      <Trend userId={id} />
      {/* A manager writes on this person's days from here - the only place a
          note is written, because it is the only screen that is about one
          person's days and not the manager's own (ADR 0021). */}
      <WeekView title={() => name ?? t('team.person')} load={load} writeFor={id} />
    </div>
  )
}
