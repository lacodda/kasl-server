import { useId, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { ApiError } from '@/lib/api'
import { useSession } from '@/lib/session'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Panel } from '@/components/ui/Panel'

export function Login() {
  const { t } = useTranslation()
  const { signIn } = useSession()
  const emailId = useId()
  const passwordId = useId()

  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  async function onSubmit(event: FormEvent) {
    event.preventDefault()
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

          <Button type="submit" variant="primary" size="lg" className="w-full" disabled={submitting}>
            {submitting ? t('login.submitting') : t('login.submit')}
          </Button>
        </form>
      </Panel>
    </div>
  )
}
