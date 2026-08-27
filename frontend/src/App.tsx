import { useEffect, useState } from 'react'
import { Navigate, Route, Routes } from 'react-router'
import { useTranslation } from 'react-i18next'
import { api } from '@/lib/api'
import { useSession } from '@/lib/session'
import { Login } from '@/pages/Login'
import { Privacy } from '@/pages/Privacy'
import { Button } from '@/components/ui/Button'

export function App() {
  const { t } = useTranslation()
  const { user } = useSession()

  // The very first paint, before `me()` has answered. Rendering the login
  // screen here would flash it at someone who is already signed in.
  if (user === undefined) {
    return <div className="flex min-h-full items-center justify-center text-sm text-dim">{t('common.loading')}</div>
  }

  if (!user) return <Login />

  return (
    <div className="flex min-h-full flex-col">
      <Header />
      <main className="flex-1 p-6">
        <Routes>
          <Route path="/privacy" element={<Privacy />} />
          {/* One screen so far. Anything else lands on it rather than on a
              blank page - the rest of the map arrives with v0.12 and v0.13. */}
          <Route path="*" element={<Navigate to="/privacy" replace />} />
        </Routes>
      </main>
    </div>
  )
}

function Header() {
  const { t } = useTranslation()
  const { user, signOut } = useSession()
  const version = useServerVersion()

  return (
    <header className="flex items-center justify-between border-b border-line px-6 py-3">
      <div className="flex items-baseline gap-3">
        <span className="font-mono text-sm font-semibold text-accent-2">{t('app.name')}</span>
        {/* The server's version, not this bundle's: the UI ships inside the
            binary, and two numbers on one product send bug reports to the
            wrong place. */}
        <span className="font-mono text-xs text-faint tabular">{version}</span>
      </div>
      <div className="flex items-center gap-4">
        <span className="text-sm text-dim">{user?.display_name}</span>
        <Button size="sm" onClick={() => void signOut()}>
          {t('nav.signOut')}
        </Button>
      </div>
    </header>
  )
}

/**
 * The running server's version.
 *
 * Empty until `/health` answers, and empty if it does not: a header that says
 * nothing is better than one that states a version nobody verified.
 */
function useServerVersion() {
  const [version, setVersion] = useState('')

  useEffect(() => {
    let cancelled = false
    api
      .health()
      .then((health) => {
        if (!cancelled) setVersion(health.version)
      })
      .catch(() => {
        // The page is already open, so the server was reachable a moment ago.
        // Nothing here is worth interrupting the user for.
      })
    return () => {
      cancelled = true
    }
  }, [])

  return version
}
