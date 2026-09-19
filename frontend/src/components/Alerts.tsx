import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import { BellOff, Clock, Hourglass, TriangleAlert } from 'lucide-react'
import { api, type Alert, type AlertRule, type AlertsResponse } from '@/lib/api'
import { alertPhrase, alertTone, isOpen } from '@/lib/alerts'
import { Panel } from '@/components/ui/panel'
import { Button } from '@/components/ui/button'

/**
 * What the server noticed on its own, at the top of the dashboard.
 *
 * Above the signals band, and that order is the whole claim of this screen.
 * A signal says "somebody's hours have been sliding for three weeks" - true,
 * useful, and it will still be true tomorrow. An alert says "this machine has
 * said nothing for thirty hours", which stops being actionable the longer it
 * waits. The thing that decays goes first.
 *
 * Every row states figures and never conclusions, exactly as the signals do:
 * a day that ran to twelve hours is a release, a crisis, or somebody who
 * forgot to close kasl, and this screen has no business guessing which. What
 * it adds is a way to say "I looked" - without which a manager reads the same
 * four rows every morning and stops reading them by Thursday.
 */
export function Alerts() {
  const { t } = useTranslation()
  const [loaded, setLoaded] = useState<AlertsResponse | null>(null)
  const [failed, setFailed] = useState(false)
  const [answering, setAnswering] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    api
      .alerts()
      .then((answer) => {
        if (!cancelled) setLoaded(answer)
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const acknowledge = useCallback(async (id: string) => {
    setAnswering(id)
    try {
      await api.acknowledgeAlert(id)
      // Dropped from the list rather than re-fetched. The row is gone from
      // this reader's open feed either way, and a refetch would re-run the
      // whole query to remove one line somebody is watching.
      setLoaded((current) =>
        current ? { ...current, alerts: current.alerts.filter((alert) => alert.id !== id), open: Math.max(0, current.open - 1) } : current,
      )
    } catch {
      // Already answered, or it resolved itself between the page loading and
      // the click. Either way the row no longer belongs in an open feed, and
      // an error banner over a thing that is simply finished would be noise.
      setLoaded((current) => (current ? { ...current, alerts: current.alerts.filter((alert) => alert.id !== id) } : current))
    } finally {
      setAnswering(null)
    }
  }, [])

  // A band that failed to load says nothing rather than "all clear": the
  // reassuring reading of an error is the wrong one, and the screens below are
  // unaffected either way.
  if (failed || !loaded) return null

  const open = loaded.alerts.filter(isOpen)

  if (open.length === 0) {
    return (
      <Panel className="flex items-center gap-2 px-5 py-3.5">
        <BellOff className="size-3.5 shrink-0 text-faint" />
        <p className="text-sm text-dim">
          {/* Says how many people the sweep watches. "Nothing open" alone
              cannot be told from "nobody is being watched", and only one of
              those is good news - the same rule the signals band follows. */}
          {t('alerts.nothing', { count: loaded.people })}
        </p>
      </Panel>
    )
  }

  return (
    <Panel className="divide-y divide-line">
      <div className="flex items-baseline justify-between gap-3 px-5 pb-2 pt-3.5">
        <h2 className="text-xs font-medium text-dim">{t('alerts.title')}</h2>
        {/* How many, not at what threshold. A caption reading "silence after
            12 h" was what this said first, and seeing it on the screen showed
            why it cannot: there are three thresholds, one per rule, and a band
            of four rows raised at three different numbers under a heading
            quoting one of them states something false about three of them.
            Each row already carries the figure it was measured against, in
            its own sentence, where it belongs. */}
        <span className="text-2xs text-faint tabular">{t('alerts.count', { count: open.length })}</span>
      </div>
      {open.map((alert) => (
        <AlertRow key={alert.id} alert={alert} onAcknowledge={acknowledge} answering={answering === alert.id} />
      ))}
    </Panel>
  )
}

/** The colour roles as literal classes Tailwind can find in the source. */
const TONE_CLASS = {
  warn: 'text-warn',
  info: 'text-info',
} as const

/** The icon each rule wears. */
const ICON: Record<AlertRule, typeof TriangleAlert> = {
  no_agent_data: TriangleAlert,
  overwork: Hourglass,
  day_not_closed: Clock,
}

function AlertRow({
  alert,
  onAcknowledge,
  answering,
}: {
  alert: Alert
  onAcknowledge: (id: string) => void
  answering: boolean
}) {
  const { t } = useTranslation()
  const phrase = alertPhrase(alert)
  const Icon = ICON[alert.rule]

  return (
    <div className="flex items-start gap-3 px-4 py-2.5 sm:items-center sm:px-5">
      {/* The icon carries the meaning alongside the colour: this product's
          accent is a gold that sits 3.7 ΔE from `warn`, so a row told apart by
          hue alone is told apart by nothing. */}
      <span className={`mt-0.5 shrink-0 sm:mt-0 ${TONE_CLASS[alertTone(alert.rule)]}`} aria-hidden>
        <Icon className="size-3.5" />
      </span>
      {/* The name and the sentence stack on a phone. Side by side they left
          the sentence about eighty pixels, truncated - and the sentence is the
          whole alert. */}
      <Link
        to={`/team/${alert.user_id}`}
        className="flex min-w-0 flex-1 flex-col gap-0.5 rounded-[9px] transition-colors hover:text-text sm:flex-row sm:items-center sm:gap-3"
      >
        <span className="min-w-0 truncate text-sm font-medium sm:w-44 sm:shrink-0">{alert.display_name}</span>
        <span className="min-w-0 text-sm text-dim sm:flex-1 sm:truncate">{t(phrase.key, phrase.values)}</span>
      </Link>
      {alert.department && <span className="hidden shrink-0 text-[11px] text-faint sm:block">{alert.department}</span>}
      {/* Answering it is a button and not a link, because it changes
          something. Outside the `Link` above, so the ordinary click on the row
          still goes to the person: the common action is looking, and a screen
          where the easy gesture dismisses is a screen that trains people to
          dismiss. */}
      <Button
        variant="ghost"
        size="sm"
        className="shrink-0"
        disabled={answering}
        onClick={() => onAcknowledge(alert.id)}
        title={t('alerts.acknowledgeHint')}
      >
        {t('alerts.acknowledge')}
      </Button>
    </div>
  )
}
