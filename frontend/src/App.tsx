import { useEffect, useState } from 'react'
import { NavLink, Navigate, Route, Routes } from 'react-router'
import { useTranslation } from 'react-i18next'
import { CalendarCheck, CalendarDays, CalendarRange, ShieldCheck, Users } from 'lucide-react'
import { api } from '@/lib/api'
import { useSession } from '@/lib/session'
import { Calendar } from '@/pages/Calendar'
import { Dashboard, PersonWeek } from '@/pages/Dashboard'
import { Heatmap } from '@/pages/Heatmap'
import { Login } from '@/pages/Login'
import { MyDay } from '@/pages/MyDay'
import { Privacy } from '@/pages/Privacy'
import { Webhooks } from '@/pages/Webhooks'
import { Button } from '@/components/ui/button'
import { destinations, type Destination } from '@/lib/navigation'

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
  const sections = destinations(managesPeople)

  return (
    <div className="flex min-h-full flex-col">
      {health.demo && <DemoBanner />}
      <Header destinations={sections} version={health.version} />
      {/* The bottom padding is the height of the phone's navigation bar plus
          the home indicator's inset: without it the last row of every screen
          sits under the bar with no way to scroll out from under it. It is
          dropped at `sm`, where the bar is gone and the tabs are in the
          header. */}
      <main className="flex-1 px-4 pt-4 pb-[calc(env(safe-area-inset-bottom)+4.5rem)] sm:p-6">
        <Routes>
          <Route path="/day" element={<MyDay />} />
          {/* Guarded on the server too - these routes answer 403 to an
              employee. Hiding them here is for tidiness, not for safety. */}
          {managesPeople && <Route path="/team" element={<Dashboard />} />}
          {managesPeople && <Route path="/month" element={<Heatmap />} />}
          {managesPeople && <Route path="/team/:id" element={<PersonWeek />} />}
          {/* No tab: the phone's bar holds five screens and has five. Reached
              from the alerts band, next to what it delivers. */}
          {user.role === 'admin' && <Route path="/webhooks" element={<Webhooks />} />}
          {/* Readable by everyone signed in; the screen hides its controls
              from anyone the server would refuse. */}
          <Route path="/calendar" element={<Calendar />} />
          <Route path="/privacy" element={<Privacy />} />
          {/* An unknown path lands on the person's own week rather than on a
              blank page. The manager's screens arrive with v0.13. */}
          <Route path="*" element={<Navigate to="/day" replace />} />
        </Routes>
      </main>
      <BottomNav destinations={sections} />
    </div>
  )
}

/** The icon each destination wears, by the name the list gives it. */
const ICON = {
  day: CalendarDays,
  team: Users,
  month: CalendarRange,
  calendar: CalendarCheck,
  privacy: ShieldCheck,
} as const

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

function Header({ destinations, version }: { destinations: Destination[]; version: string }) {
  const { t } = useTranslation()
  const { user, signOut } = useSession()

  return (
    <header className="flex items-center justify-between gap-3 border-b border-line px-4 py-3 sm:px-6">
      <div className="flex min-w-0 items-baseline gap-3">
        <span className="shrink-0 font-mono text-sm font-semibold text-accent-2">{t('app.name')}</span>
        {/* The server's version, not this bundle's: the UI ships inside the
            binary, and two numbers on one product send bug reports to the
            wrong place. Off the phone's header, where this row has to hold a
            name and a sign-out button at 320px. */}
        <span className="hidden font-mono text-xs text-faint tabular sm:inline">{version}</span>
        <nav className="ml-4 hidden items-center gap-1 sm:flex">
          {destinations.map((destination) => (
            <Tab key={destination.to} to={destination.to}>
              {t(destination.label)}
            </Tab>
          ))}
        </nav>
      </div>
      <div className="flex min-w-0 items-center gap-2 sm:gap-4">
        {/* Truncated rather than hidden: whose page this is matters more on a
            phone that gets handed around than on a desktop, and a long name
            must not push the sign-out button off the screen. */}
        <span className="min-w-0 truncate text-sm text-dim">{user?.display_name}</span>
        <Button size="sm" className="shrink-0" onClick={() => void signOut()}>
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
 * The phone's navigation: the same destinations, along the bottom.
 *
 * A bar rather than a menu behind a button. A menu is a second thing to open
 * before you can go anywhere, and it brings state, a focus trap and a way to
 * be left open across a route change; this product has three or four screens,
 * which fit across a phone as they are. It also puts them where the thumb
 * already is, which a menu button in the top corner never is.
 *
 * Fixed rather than sitting at the end of the flow: the destinations have to
 * be reachable from the middle of a long month without scrolling to the bottom
 * of it first. `main` carries the matching padding so nothing hides beneath.
 */
function BottomNav({ destinations }: { destinations: Destination[] }) {
  const { t } = useTranslation()

  return (
    <nav
      aria-label={t('nav.sections')}
      className="fixed inset-x-0 bottom-0 z-20 flex border-t border-line bg-raise pb-[env(safe-area-inset-bottom)] sm:hidden"
    >
      {destinations.map(({ to, label, icon }) => {
        const Icon = ICON[icon]
        return (
          <NavLink
            key={to}
            to={to}
            // The whole cell is the target rather than the text inside it, so
            // a destination is hit without aiming. Four of them still clear
            // the 44px minimum at 320px wide.
            className={({ isActive }) =>
              `flex min-h-14 min-w-0 flex-1 flex-col items-center justify-center gap-1 px-1 py-2 text-[11px] transition-colors ${
                isActive ? 'text-accent-2' : 'text-dim'
              }`
            }
          >
            {/* The labels here are the short names, not the screens' own
                headings: "What is stored about me" is a title and a tab both,
                and as a tab it truncated to "What is stored a…". The page
                keeps the sentence; the tab says "My data" and the icon
                carries the rest. */}
            <Icon className="size-4 shrink-0" />
            <span className="w-full truncate text-center">{t(label)}</span>
          </NavLink>
        )
      })}
    </nav>
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
