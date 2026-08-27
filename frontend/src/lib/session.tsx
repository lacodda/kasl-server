import { createContext, use, useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { api, type Identity } from '@/lib/api'

interface Session {
  /** `undefined` while the first `me()` is still in flight. */
  user: Identity | null | undefined
  signIn: (email: string, password: string) => Promise<void>
  signOut: () => Promise<void>
}

const SessionContext = createContext<Session | null>(null)

/**
 * Who is signed in, asked of the server rather than remembered here.
 *
 * The session lives in an HttpOnly cookie (ADR 0007), so the page cannot read
 * it: the only way to know whether it is still valid is to ask. That is also
 * why a reload is not a sign-out - the cookie survives, and `me()` answers.
 */
export function SessionProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<Identity | null | undefined>(undefined)

  useEffect(() => {
    let cancelled = false
    api
      .me()
      .then((identity) => {
        if (!cancelled) setUser(identity)
      })
      .catch(() => {
        // A 401 is the ordinary "nobody is signed in" answer, not a fault; an
        // unreachable server lands here too, and the login screen is still the
        // right place to be - it reports the failure when a sign-in is tried.
        if (!cancelled) setUser(null)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const signIn = useCallback(async (email: string, password: string) => {
    await api.login(email, password)
    // The login response carries no user, only the cookie, so the identity
    // comes from the same call every reload uses.
    setUser(await api.me())
  }, [])

  const signOut = useCallback(async () => {
    try {
      await api.logout()
    } finally {
      // Whatever the server said, this browser is done: leaving the user
      // apparently signed in after they asked to leave is the worse failure.
      setUser(null)
    }
  }, [])

  const value = useMemo(() => ({ user, signIn, signOut }), [user, signIn, signOut])
  return <SessionContext value={value}>{children}</SessionContext>
}

export function useSession() {
  const session = use(SessionContext)
  if (!session) throw new Error('useSession must be used inside a SessionProvider')
  return session
}
