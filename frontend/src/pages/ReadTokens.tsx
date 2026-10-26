import { useCallback, useEffect, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { Copy, KeyRound } from 'lucide-react'
import { api, type IssuedReadToken, type ReadScope, type ReadToken } from '@/lib/api'
import { moment } from '@/lib/day'
import { DATASETS, datasetUrl, metricsUrl, scrapeConfig, tokenState, type TokenState } from '@/lib/read-tokens'
import { useSession } from '@/lib/session'
import { Panel, SectionLabel } from '@/components/ui/panel'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Segment, SegmentedControl } from '@/components/ui/segmented-control'

/** Where the step-by-step guides live, one per scope. */
const GUIDE: Record<ReadScope, string> = {
  hours: 'https://lacodda.github.io/kasl-server/guides/reading-hours-into-grafana/',
  metrics: 'https://lacodda.github.io/kasl-server/guides/watching-the-server-with-prometheus/',
}

/** What each scope's token is usually named after. */
const PLACEHOLDER: Record<ReadScope, string> = {
  hours: 'Grafana',
  metrics: 'Prometheus',
}

/** The colour each state is drawn in, as literal classes Tailwind can find. */
const STATE_CLASS: Record<TokenState, string> = {
  live: 'text-good',
  unused: 'text-dim',
  revoked: 'text-faint',
}

/**
 * Read tokens: how a dashboard tool reads the hours this person can see
 * (ADR 0025) - and, for an administrator, how Prometheus reads the server's
 * own health (ADR 0026).
 *
 * Reached from the export menu rather than from the navigation. A token is an
 * export that keeps going - the same hours, read from somewhere else - and the
 * menu that downloads them once is where somebody looking for "get this into
 * Grafana" already is.
 *
 * Issuing shows the token once, with the address to point a tool at; the list
 * says which tokens are being read with, so one nobody uses any more can be
 * revoked rather than left lying in a config file.
 */
export function ReadTokens() {
  const { t } = useTranslation()
  const { user } = useSession()
  const isAdmin = user?.role === 'admin'
  const [tokens, setTokens] = useState<ReadToken[] | null>(null)
  const [failed, setFailed] = useState(false)
  const [issued, setIssued] = useState<IssuedReadToken | null>(null)

  const load = useCallback(() => {
    api
      .readTokens()
      .then((value) => {
        setTokens(value)
        setFailed(false)
      })
      .catch(() => setFailed(true))
  }, [])

  useEffect(() => load(), [load])

  if (failed) return <p className="text-sm text-bad">{t('common.error')}</p>
  if (!tokens) return <p className="text-sm text-dim">{t('common.loading')}</p>

  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <div>
        <h1 className="text-lg font-semibold">{t('readTokens.title')}</h1>
        <p className="mt-2 text-sm text-dim">{t('readTokens.subtitle')}</p>
        {isAdmin && <p className="mt-2 text-sm text-dim">{t('readTokens.subtitleMetrics')}</p>}
      </div>

      <IssueForm
        mayReadMetrics={isAdmin}
        onIssued={(token) => {
          setIssued(token)
          load()
        }}
      />
      {issued && <IssuedCard issued={issued} />}

      <Panel className="divide-y divide-line">
        <div className="px-4 pb-2 pt-3.5 sm:px-5">
          <SectionLabel>{isAdmin ? t('readTokens.everyone') : t('readTokens.yours')}</SectionLabel>
        </div>
        {tokens.length === 0 ? (
          <p className="px-4 py-3.5 text-sm text-dim sm:px-5">{t('readTokens.none')}</p>
        ) : (
          tokens.map((token) => <TokenRow key={token.id} token={token} showOwner={token.owner_id !== user?.id} onRevoked={load} />)
        )}
      </Panel>

      <div className="flex flex-col items-start gap-1.5">
        <Guide scope="hours" />
        {isAdmin && <Guide scope="metrics" />}
      </div>
    </div>
  )
}

function Guide({ scope }: { scope: ReadScope }) {
  const { t } = useTranslation()
  return (
    <a href={GUIDE[scope]} className="text-sm text-accent-2 underline-offset-2 hover:underline" target="_blank" rel="noreferrer">
      {t(`readTokens.guide.${scope}`)}
    </a>
  )
}

/**
 * Names a token and, for an administrator, what it reads. A manager's token
 * reads the hours and nothing else, so they are not asked.
 */
function IssueForm({ mayReadMetrics, onIssued }: { mayReadMetrics: boolean; onIssued: (token: IssuedReadToken) => void }) {
  const { t } = useTranslation()
  const [name, setName] = useState('')
  const [scope, setScope] = useState<ReadScope>('hours')
  const [state, setState] = useState<'idle' | 'issuing' | 'failed'>('idle')

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!name.trim()) return
    setState('issuing')
    try {
      onIssued(await api.issueReadToken(name.trim(), scope))
      setName('')
      setState('idle')
    } catch {
      setState('failed')
    }
  }

  return (
    <Panel className="space-y-3 p-4 sm:p-5">
      {mayReadMetrics && (
        <div className="space-y-1">
          <span id="token-scope" className="block text-xs text-faint">
            {t('readTokens.scopeLabel')}
          </span>
          <SegmentedControl aria-labelledby="token-scope" value={scope} onValueChange={(next) => setScope(next === 'metrics' ? 'metrics' : 'hours')}>
            <Segment value="hours">{t('readTokens.scope.hours')}</Segment>
            <Segment value="metrics">{t('readTokens.scope.metrics')}</Segment>
          </SegmentedControl>
        </div>
      )}
      <form className="flex flex-col gap-2 sm:flex-row sm:items-end" onSubmit={(event) => void submit(event)}>
        <label className="min-w-0 flex-1 space-y-1">
          <span className="block text-xs text-faint">{t('readTokens.nameLabel')}</span>
          <Input value={name} onChange={(event) => setName(event.target.value)} placeholder={PLACEHOLDER[scope]} maxLength={100} />
        </label>
        <Button type="submit" variant="primary" disabled={state === 'issuing' || !name.trim()}>
          <KeyRound className="size-3.5" aria-hidden />
          {t('readTokens.issue')}
        </Button>
      </form>
      {state === 'failed' && (
        <p role="alert" className="text-sm text-bad">
          {t('common.error')}
        </p>
      )}
    </Panel>
  )
}

/**
 * The one time the token is shown, and where to point a tool with it.
 *
 * In a read-only field rather than as text: a field selects whole on focus,
 * which is the way to copy it where the clipboard is refused - a server
 * reached over plain `http://` on a home network is not a secure context, and
 * the browser withholds the clipboard from it.
 */
function IssuedCard({ issued }: { issued: IssuedReadToken }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState<'no' | 'yes' | 'select'>('no')
  const origin = window.location.origin

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(issued.token)
      setCopied('yes')
    } catch {
      setCopied('select')
    }
  }

  return (
    <Panel className="space-y-3 border-accent p-4 sm:p-5">
      <p className="text-sm font-medium">{t('readTokens.issued', { name: issued.name })}</p>
      <div className="flex gap-2">
        <Input
          readOnly
          value={issued.token}
          aria-label={t('readTokens.tokenLabel')}
          className="font-mono text-xs"
          onFocus={(event) => event.currentTarget.select()}
        />
        <Button onClick={() => void copy()} aria-label={t('readTokens.copy')}>
          <Copy className="size-3.5" aria-hidden />
          <span className="hidden sm:inline">{t('readTokens.copy')}</span>
        </Button>
      </div>
      <p className="text-xs text-dim">
        {copied === 'yes' ? t('readTokens.copied') : copied === 'select' ? t('readTokens.selectToCopy') : t('readTokens.shownOnce')}
      </p>

      {issued.scope === 'metrics' ? (
        <div className="space-y-1.5">
          <p className="text-xs text-faint">{t('readTokens.scrapeAt')}</p>
          <p className="break-all rounded-inner bg-soft px-2.5 py-1.5 font-mono text-2xs text-dim">{metricsUrl(origin)}</p>
          <p className="pt-1 text-xs text-faint">{t('readTokens.scrapeJob')}</p>
          <pre className="overflow-x-auto rounded-inner bg-soft px-2.5 py-1.5 font-mono text-2xs text-dim">{scrapeConfig(origin)}</pre>
        </div>
      ) : (
        <div className="space-y-1.5">
          <p className="text-xs text-faint">{t('readTokens.pointAt')}</p>
          <ul className="space-y-1">
            {DATASETS.map((dataset) => (
              <li key={dataset} className="break-all rounded-inner bg-soft px-2.5 py-1.5 font-mono text-2xs text-dim">
                {datasetUrl(origin, dataset)}
              </li>
            ))}
          </ul>
          <p className="pt-1 text-xs text-faint">{t('readTokens.header')}</p>
          <p className="break-all rounded-inner bg-soft px-2.5 py-1.5 font-mono text-2xs text-dim">Authorization: Bearer {issued.token}</p>
        </div>
      )}
    </Panel>
  )
}

function TokenRow({ token, showOwner, onRevoked }: { token: ReadToken; showOwner: boolean; onRevoked: () => void }) {
  const { t } = useTranslation()
  const [confirming, setConfirming] = useState(false)
  const [failed, setFailed] = useState(false)
  const state = tokenState(token)

  const revoke = async () => {
    try {
      await api.revokeReadToken(token.id)
      onRevoked()
    } catch {
      setFailed(true)
    }
  }

  return (
    <div className="flex flex-col gap-2 px-4 py-3 text-sm sm:flex-row sm:items-center sm:gap-3 sm:px-5">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className={state === 'revoked' ? 'text-dim line-through' : 'font-medium'}>{token.name}</span>
          <span className="text-xs text-faint">{t(`readTokens.scope.${token.scope}`)}</span>
          {showOwner && <span className="text-xs text-dim">{token.owner}</span>}
        </div>
        <p className="mt-0.5 text-xs text-faint tabular">
          {t('readTokens.created', { at: moment(token.created_at) })}
          {' · '}
          <span className={STATE_CLASS[state]}>
            {state === 'revoked' && token.revoked_at
              ? t('readTokens.revokedAt', { at: moment(token.revoked_at) })
              : token.last_used_at
                ? t('readTokens.lastUsed', { at: moment(token.last_used_at) })
                : t('readTokens.neverUsed')}
          </span>
        </p>
        {failed && <p className="mt-1 text-xs text-bad">{t('common.error')}</p>}
      </div>
      {state !== 'revoked' &&
        (confirming ? (
          <div className="flex shrink-0 gap-2">
            <Button size="sm" variant="danger" onClick={() => void revoke()}>
              {t('readTokens.confirmRevoke')}
            </Button>
            <Button size="sm" onClick={() => setConfirming(false)}>
              {t('readTokens.keep')}
            </Button>
          </div>
        ) : (
          <Button size="sm" className="shrink-0 self-start sm:self-auto" onClick={() => setConfirming(true)}>
            {t('readTokens.revoke')}
          </Button>
        ))}
    </div>
  )
}
