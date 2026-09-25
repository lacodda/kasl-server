import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Send } from 'lucide-react'
import { api, type WebhookDelivery, type WebhookDestination, type WebhooksOverview } from '@/lib/api'
import { moment } from '@/lib/day'
import { deliveryState, deliveryTone, destinationProblem } from '@/lib/webhooks'
import { Panel, SectionLabel } from '@/components/ui/panel'
import { Button } from '@/components/ui/button'

/** Where the guide to setting destinations up lives. */
const GUIDE = 'https://lacodda.github.io/kasl-server/guides/sending-alerts-to-a-chat/'

/** How long after a test to look again: the dispatcher's tick, and a margin. */
const RECHECK_AFTER_TEST_MS = 7000

/**
 * Where the server sends what it notices, and how the last ones went.
 *
 * Read-only, and that is the design rather than a gap: a destination is a
 * credential to post as somebody, so it is declared in the server's
 * environment, next to the database password (ADR 0019). What this screen can
 * do is what an operator needs after setting one - see that it is there, see
 * that it works, and send a test to be sure.
 *
 * Reached from the alerts band rather than from the navigation: the bar along
 * a phone's bottom holds five screens and has five, and this is a screen an
 * administrator opens once a month, next to the thing whose delivery it is.
 */
export function Webhooks() {
  const { t } = useTranslation()
  const [overview, setOverview] = useState<WebhooksOverview | null>(null)
  const [failed, setFailed] = useState(false)

  const load = useCallback(() => {
    api
      .webhooks()
      .then((value) => {
        setOverview(value)
        setFailed(false)
      })
      .catch(() => setFailed(true))
  }, [])

  useEffect(() => load(), [load])

  if (failed) return <p className="text-sm text-bad">{t('common.error')}</p>
  if (!overview) return <p className="text-sm text-dim">{t('common.loading')}</p>

  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <div>
        <h1 className="text-lg font-semibold">{t('webhooks.title')}</h1>
        <p className="mt-2 text-sm text-dim">{t('webhooks.subtitle')}</p>
      </div>

      {overview.destinations.length === 0 ? (
        <Panel className="space-y-2 p-4 sm:p-5">
          <p className="text-sm">{t('webhooks.none')}</p>
          <p className="text-sm text-dim">{t('webhooks.noneHint')}</p>
          <pre className="overflow-x-auto rounded-inner bg-soft px-3 py-2 font-mono text-xs text-dim">
            KASL_WEBHOOK_TEAM=slack https://hooks.slack.com/services/…
          </pre>
          <a href={GUIDE} className="inline-block text-sm text-accent-2 underline-offset-2 hover:underline" target="_blank" rel="noreferrer">
            {t('webhooks.guide')}
          </a>
        </Panel>
      ) : (
        <div className="space-y-3">
          {overview.destinations.map((destination) => (
            <DestinationCard key={destination.name} destination={destination} onTested={() => window.setTimeout(load, RECHECK_AFTER_TEST_MS)} />
          ))}
          {/* Said, because the fix is a setting the operator would not guess:
              messages without a link are plain text by design until the
              server knows its own address. */}
          {!overview.links && <p className="text-xs text-faint">{t('webhooks.noLinks')}</p>}
        </div>
      )}

      <Panel className="divide-y divide-line">
        <div className="flex items-baseline justify-between gap-3 px-4 pb-2 pt-3.5 sm:px-5">
          <SectionLabel>{t('webhooks.recent')}</SectionLabel>
          <Button variant="ghost" size="sm" onClick={load}>
            {t('webhooks.refresh')}
          </Button>
        </div>
        {overview.recent.length === 0 ? (
          <p className="px-4 py-3.5 text-sm text-dim sm:px-5">{t('webhooks.nothingSent')}</p>
        ) : (
          overview.recent.map((delivery) => <DeliveryRow key={delivery.id} delivery={delivery} />)
        )}
      </Panel>
    </div>
  )
}

/** The colour roles as literal classes Tailwind can find in the source. */
const TONE_CLASS = {
  good: 'text-good',
  info: 'text-info',
  warn: 'text-warn',
  bad: 'text-bad',
} as const

function DestinationCard({ destination, onTested }: { destination: WebhookDestination; onTested: () => void }) {
  const { t } = useTranslation()
  const [testing, setTesting] = useState<'idle' | 'sending' | 'queued' | 'failed'>('idle')
  const problem = destinationProblem(destination)

  const test = async () => {
    setTesting('sending')
    try {
      await api.testWebhook(destination.name)
      setTesting('queued')
      onTested()
    } catch {
      setTesting('failed')
    }
  }

  return (
    <Panel className="space-y-3 p-4 sm:p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-baseline gap-2">
            <span className="font-mono text-sm font-semibold">{destination.name}</span>
            <span className="rounded-[9px] bg-soft px-2 py-0.5 font-mono text-2xs text-dim">{destination.kind}</span>
          </div>
          <p className="mt-1 truncate font-mono text-xs text-faint">{destination.target}</p>
        </div>
        <Button variant="ghost" size="sm" disabled={testing === 'sending'} onClick={() => void test()}>
          <Send className="size-3.5" aria-hidden />
          {t('webhooks.sendTest')}
        </Button>
      </div>

      <dl className="grid grid-cols-1 gap-x-6 gap-y-2 text-sm sm:grid-cols-2">
        <div>
          <dt className="text-xs text-faint">{t('webhooks.hears')}</dt>
          <dd className="mt-0.5 font-mono text-xs text-dim">{destination.events.join(', ')}</dd>
        </div>
        <div>
          <dt className="text-xs text-faint">{t('webhooks.about')}</dt>
          <dd className="mt-0.5 text-dim">{destination.department ?? t('webhooks.everyone')}</dd>
        </div>
        <div className="sm:col-span-2">
          <dt className="text-xs text-faint">{t('webhooks.sofar')}</dt>
          <dd className="mt-0.5 text-dim tabular">
            {t('webhooks.counts', { delivered: destination.delivered, pending: destination.pending, abandoned: destination.abandoned })}
            {destination.last_delivered_at && <> · {t('webhooks.lastDelivered', { at: moment(destination.last_delivered_at) })}</>}
          </dd>
        </div>
      </dl>

      {problem === 'departmentMissing' && (
        <p className="text-sm text-bad">{t('webhooks.departmentMissing', { department: destination.department })}</p>
      )}
      {problem === 'failing' && (
        <p className="break-words text-sm text-bad">
          {t('webhooks.lastError', { at: destination.last_error_at ? moment(destination.last_error_at) : '—' })} {destination.last_error}
        </p>
      )}
      {testing === 'queued' && <p className="text-sm text-dim">{t('webhooks.testQueued')}</p>}
      {testing === 'failed' && <p className="text-sm text-bad">{t('common.error')}</p>}
    </Panel>
  )
}

function DeliveryRow({ delivery }: { delivery: WebhookDelivery }) {
  const { t } = useTranslation()
  const state = deliveryState(delivery)

  return (
    <div className="flex flex-col gap-1 px-4 py-2.5 text-sm sm:flex-row sm:items-baseline sm:gap-3 sm:px-5">
      <span className="shrink-0 font-mono text-xs text-faint tabular sm:w-28">{moment(delivery.created_at)}</span>
      <span className="min-w-0 flex-1">
        <span className="font-mono text-xs">{delivery.event}</span>
        {delivery.person && <span className="text-dim"> · {delivery.person}</span>}
        <span className="text-faint"> → {delivery.destination}</span>
        {/* The receiver's own words for a failure - `404 no_service`, `403
            bot was kicked` - are the whole diagnosis, so they are shown in
            full rather than behind a hover. */}
        {state !== 'delivered' && delivery.last_error && <span className="mt-0.5 block break-words text-xs text-dim">{delivery.last_error}</span>}
      </span>
      <span className={`shrink-0 text-xs ${TONE_CLASS[deliveryTone(state)]}`}>
        {t(`webhooks.state.${state}`, { attempts: delivery.attempts, at: moment(delivery.next_attempt_at) })}
      </span>
    </div>
  )
}
