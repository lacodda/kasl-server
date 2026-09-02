import { useEffect, useId, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { ApiError, api, type DemoAccounts } from '@/lib/api'
import { useSession } from '@/lib/session'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Panel } from '@/components/ui/panel'

export function Login({ demo = false }: { demo?: boolean }) {
  const { t } = useTranslation()
  const { signIn } = useSession()
  const emailId = useId()
  const passwordId = useId()

  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)
  const accounts = useDemoAccounts(demo)

  async function submit(email: string, password: string) {
    setError(null)
    setSubmitting(true)
    try {
      await signIn(email, password)
    } catch (failure: unknown) {
      // The server refuses an unknown address and a wrong password with the
      // same answer on purpose, so the form must not guess which it was.
      // Anything else is the server being unreachable, and saying so is more
      // useful than "wrong password" to someone whose password is right.
      setError(failure instanceof ApiError && failure.isUnauthorized ? t('login.failed') : t('login.unavailable'))
      setSubmitting(false)
    }
  }

  function onSubmit(event: FormEvent) {
    event.preventDefault()
    void submit(email, password)
  }

  // One click to a dashboard. The fields are filled too, so what just
  // happened is visible rather than magic.
  function tryAs(account: DemoAccounts['accounts'][number]) {
    if (!accounts) return
    setEmail(account.email)
    setPassword(accounts.password)
    void submit(account.email, accounts.password)
  }

  return (
    <div className="flex min-h-full items-center justify-center p-6">
      <Panel className="w-full max-w-sm p-7 shadow-[var(--shadow-raise)]">
        <h1 className="text-lg font-semibold">{t('login.title')}</h1>
        <p className="mt-1 text-sm text-dim">{t('login.subtitle')}</p>

        <form onSubmit={onSubmit} className="mt-6 space-y-4">
          <div className="space-y-1.5">
            <label htmlFor={emailId} className="block text-xs font-medium text-dim">
              {t('login.email')}
            </label>
            <Input
              id={emailId}
              type="email"
              value={email}
              autoComplete="username"
              autoFocus
              required
              onChange={(event) => setEmail(event.target.value)}
            />
          </div>

          <div className="space-y-1.5">
            <label htmlFor={passwordId} className="block text-xs font-medium text-dim">
              {t('login.password')}
            </label>
            <Input
              id={passwordId}
              type="password"
              value={password}
              autoComplete="current-password"
              required
              onChange={(event) => setPassword(event.target.value)}
            />
          </div>

          {error && (
            // `role="alert"` so the failure is announced, not only drawn: a
            // screen reader user gets no signal from a red line appearing.
            <p role="alert" className="rounded-[9px] bg-bad-soft px-3 py-2 text-sm text-bad">
              {error}
            </p>
          )}

          <Button type="submit" variant="primary" className="h-10 w-full" disabled={submitting}>
            {submitting ? t('login.submitting') : t('login.submit')}
          </Button>
        </form>

        {accounts && (
          <div className="mt-6 border-t border-line pt-5">
            <p className="text-xs font-medium text-dim">{t('demo.tryAs')}</p>
            <div className="mt-2 flex flex-wrap gap-2">
              {accounts.accounts.map((account) => (
                <Button key={account.email} size="sm" disabled={submitting} onClick={() => tryAs(account)}>
                  {t(`demo.roles.${account.role}`)}
                </Button>
              ))}
            </div>
            <p className="mt-3 text-xs text-faint">{t('demo.password', { password: accounts.password })}</p>
          </div>
        )}
      </Panel>
    </div>
  )
}

/**
 * The accounts a demo offers, or nothing on a real server.
 *
 * Asked only when `/health` already said this is a demo: the endpoint answers
 * 404 everywhere else, and a login page that fired a failing request at every
 * real installation would be noise in every operator's log.
 */
function useDemoAccounts(demo: boolean) {
  const [accounts, setAccounts] = useState<DemoAccounts | null>(null)

  useEffect(() => {
    if (!demo) return
    let cancelled = false
    api
      .demoAccounts()
      .then((answer) => {
        if (!cancelled) setAccounts(answer)
      })
      .catch(() => {
        // Without the list the form still works; the README has the logins.
      })
    return () => {
      cancelled = true
    }
  }, [demo])

  return accounts
}
