import { createContext, use, useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { api, isSignedOut, type Identity } from '@/lib/api'

/** How long to wait before asking a server that did not answer again. */
const RETRY_MS = 10_000

interface Session {
  /**
   * `undefined` until the server has said who is signed in - while the first
   * `me()` is in flight, and for as long as the server cannot answer.
   */
  user: Identity | null | undefined
  /**
   * The server could not say who is signed in: it is unreachable or failing,
   * which is not the same answer as "nobody". Asked again on its own every
   * few seconds, and at once by `retry`.
   */
  unreachable: boolean
  retry: () => void
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
  const [unreachable, setUnreachable] = useState(false)
  // Bumped to ask again: by the button, and by the timer while unreachable.
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    let cancelled = false
    api
      .me()
      .then((identity) => {
        if (cancelled) return
        setUnreachable(false)
        setUser(identity)
      })
      .catch((failure: unknown) => {
        if (cancelled) return
        // Only a 401 means nobody is signed in. Anything else - a 500 while
        // the database is down, a network that is not there - leaves the
        // question unanswered rather than answering it "nobody".
        if (isSignedOut(failure)) {
          setUnreachable(false)
          setUser(null)
        } else {
          setUnreachable(true)
        }
      })
    return () => {
      cancelled = true
    }
  }, [attempt])

  useEffect(() => {
    if (!unreachable) return
    const timer = setTimeout(() => setAttempt((count) => count + 1), RETRY_MS)
    return () => clearTimeout(timer)
  }, [unreachable, attempt])

  const retry = useCallback(() => setAttempt((count) => count + 1), [])

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

  const value = useMemo(() => ({ user, unreachable, retry, signIn, signOut }), [user, unreachable, retry, signIn, signOut])
  return <SessionContext value={value}>{children}</SessionContext>
}

export function useSession() {
  const session = use(SessionContext)
  if (!session) throw new Error('useSession must be used inside a SessionProvider')
  return session
}
