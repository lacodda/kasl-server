import { useEffect, useState } from 'react'
import { NavLink, Navigate, Route, Routes } from 'react-router'
import { useTranslation } from 'react-i18next'
import { api } from '@/lib/api'
import { useSession } from '@/lib/session'
import { Dashboard, PersonWeek } from '@/pages/Dashboard'
import { Heatmap } from '@/pages/Heatmap'
import { Login } from '@/pages/Login'
import { MyDay } from '@/pages/MyDay'
import { Privacy } from '@/pages/Privacy'
import { Button } from '@/components/ui/button'

export function App() {
  const { t } = useTranslation()
  const { user } = useSession()
  const health = useServerHealth()

  // The very first paint, before `me()` has answered. Rendering the login
  // screen here would flash it at someone who is already signed in.
  if (user === undefined) {
    return <div className="flex min-h-full items-center justify-center text-sm text-dim">{t('common.loading')}</div>
  }

  if (!user) {
    return (
      <div className="flex min-h-full flex-col">
        {health.demo && <DemoBanner />}
        <Login demo={health.demo} />
      </div>
    )
  }

  // Who gets the team screens. The server decides for real; this only keeps a
  // link out of the way of someone it would refuse.
  const managesPeople = user.role === 'manager' || user.role === 'admin'

  return (
    <div className="flex min-h-full flex-col">
      {health.demo && <DemoBanner />}
      <Header managesPeople={managesPeople} version={health.version} />
      <main className="flex-1 p-6">
        <Routes>
          <Route path="/day" element={<MyDay />} />
          {/* Guarded on the server too - these routes answer 403 to an
              employee. Hiding them here is for tidiness, not for safety. */}
          {managesPeople && <Route path="/team" element={<Dashboard />} />}
          {managesPeople && <Route path="/month" element={<Heatmap />} />}
          {managesPeople && <Route path="/team/:id" element={<PersonWeek />} />}
          <Route path="/privacy" element={<Privacy />} />
          {/* An unknown path lands on the person's own week rather than on a
              blank page. The manager's screens arrive with v0.13. */}
          <Route path="*" element={<Navigate to="/day" replace />} />
        </Routes>
      </main>
    </div>
  )
}

/**
 * The line that says the data is invented. On every screen, including the
 * login: a visitor who lands on a shared demo link must not mistake the team
 * for anyone's real one, and a screenshot must carry the label with it.
 */
function DemoBanner() {
  const { t } = useTranslation()
  return (
    <div role="note" className="bg-accent-soft px-4 py-1.5 text-center text-xs font-medium text-accent-2">
      {t('demo.banner')}
    </div>
  )
}

function Header({ managesPeople, version }: { managesPeople: boolean; version: string }) {
  const { t } = useTranslation()
  const { user, signOut } = useSession()

  return (
    <header className="flex items-center justify-between border-b border-line px-6 py-3">
      <div className="flex items-baseline gap-3">
        <span className="font-mono text-sm font-semibold text-accent-2">{t('app.name')}</span>
        {/* The server's version, not this bundle's: the UI ships inside the
            binary, and two numbers on one product send bug reports to the
            wrong place. */}
        <span className="font-mono text-xs text-faint tabular">{version}</span>
        {/* The employee's own screens. The mockup gives each role a sidebar;
            with two screens a header row says the same thing without the
            furniture, and the sidebar arrives when v0.13 fills it. */}
        <nav className="ml-4 flex items-center gap-1">
          <Tab to="/day">{t('nav.myDay')}</Tab>
          {managesPeople && <Tab to="/team">{t('nav.team')}</Tab>}
          {managesPeople && <Tab to="/month">{t('nav.heatmap')}</Tab>}
          <Tab to="/privacy">{t('nav.privacy')}</Tab>
        </nav>
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

/** One entry in the header navigation, lit when its screen is the open one. */
function Tab({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <NavLink
      to={to}
      className={({ isActive }) =>
        `rounded-[9px] px-2.5 py-1 text-sm transition-colors ${
          isActive ? 'bg-accent-soft text-accent-2' : 'text-dim hover:bg-soft hover:text-text'
        }`
      }
    >
      {children}
    </NavLink>
  )
}

/**
 * What the running server says about itself: its version, and whether it is
 * a demo.
 *
 * Empty until `/health` answers, and empty if it does not: a header that says
 * nothing is better than one that states a version nobody verified, and a
 * page that cannot reach the server has no business calling it a demo.
 */
function useServerHealth() {
  const [health, setHealth] = useState<{ version: string; demo: boolean }>({ version: '', demo: false })

  useEffect(() => {
    let cancelled = false
    api
      .health()
      .then((answer) => {
        if (!cancelled) setHealth({ version: answer.version, demo: answer.demo === true })
      })
      .catch(() => {
        // The page is already open, so the server was reachable a moment ago.
        // Nothing here is worth interrupting the user for.
      })
    return () => {
      cancelled = true
    }
  }, [])

  return health
}
