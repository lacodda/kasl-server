import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router'
import { TrendingDown, TrendingUp, TriangleAlert } from 'lucide-react'
import { api, type Signal, type SignalsResponse } from '@/lib/api'
import { signalPhrase, signalTone } from '@/lib/signals'
import { Panel } from '@/components/ui/panel'

/**
 * What the server thinks is worth a look, above the team table.
 *
 * On the dashboard rather than on a page of its own: a signal nobody opens is
 * not a signal, and this is where a manager already is. Each line links to the
 * person, because the next thing anybody wants after "Lukas has been sliding"
 * is the weeks that say so.
 *
 * The wording states figures and never conclusions (ADR 0016). Falling hours
 * are a holiday, a hospital, or a project that ended, and the screen has no
 * business guessing which - so it says what was measured and stops.
 */
export function Signals() {
  const { t } = useTranslation()
  const [loaded, setLoaded] = useState<SignalsResponse | null>(null)
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let cancelled = false
    api
      .teamSignals()
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

  // A band that failed to load says nothing rather than "all clear": the
  // reassuring reading of an error is the wrong one, and the table below is
  // unaffected either way.
  if (failed || !loaded) return null

  if (loaded.signals.length === 0) {
    return (
      <Panel className="px-5 py-3.5">
        <p className="text-sm text-dim">
          {/* Says how many people were examined. "Nothing found" alone cannot
              be told from "nobody was looked at", and only one of those is
              good news. */}
          {t('signals.nothing', { count: loaded.people })}
        </p>
      </Panel>
    )
  }

  return (
    <Panel className="divide-y divide-line">
      <div className="px-5 pb-2 pt-3.5">
        <h2 className="text-xs font-medium text-dim">{t('signals.title')}</h2>
      </div>
      {loaded.signals.map((signal, index) => (
        <SignalRow key={`${signal.user_id}-${signal.kind}-${index}`} signal={signal} />
      ))}
    </Panel>
  )
}

/** The colour roles as literal classes Tailwind can find in the source. */
const TONE_CLASS = {
  warn: 'text-warn',
  info: 'text-info',
} as const

function SignalRow({ signal }: { signal: Signal }) {
  const { t } = useTranslation()
  const tone = signalTone(signal.kind)
  const phrase = signalPhrase(signal)

  return (
    <Link to={`/team/${signal.user_id}`} className="flex items-center gap-3 px-5 py-2.5 transition-colors hover:bg-soft">
      <span className={`shrink-0 ${TONE_CLASS[tone]}`}>
        <Icon signal={signal} />
      </span>
      <span className="min-w-0 w-44 shrink-0 truncate text-sm font-medium">{signal.display_name}</span>
      {/* The sentence carries the figures the server measured. Nothing here
          says whether any of it is a problem. */}
      <span className="min-w-0 flex-1 truncate text-sm text-dim">{t(phrase.key, phrase.values)}</span>
      {signal.department && <span className="hidden shrink-0 text-[11px] text-faint sm:block">{signal.department}</span>}
    </Link>
  )
}

/**
 * The icon carries the meaning alongside the colour, as the mockup requires.
 *
 * The arrow follows the actual direction rather than the signal's name: an
 * unusual week can be unusually long, and an arrow pointing the wrong way
 * would contradict the sentence beside it - the kind of mismatch a reader
 * trusts over the words.
 */
function Icon({ signal }: { signal: Signal }) {
  if (signal.kind === 'no_data') return <TriangleAlert className="size-3.5" />

  const up = signal.kind === 'unusual_week' && signal.to_seconds !== null && signal.median_seconds !== null && signal.to_seconds > signal.median_seconds

  return up ? <TrendingUp className="size-3.5" /> : <TrendingDown className="size-3.5" />
}
