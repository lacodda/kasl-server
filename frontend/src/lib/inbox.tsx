/**
 * The signed-in person's inbox, shared by the badge in the header and the
 * screen that lists it.
 *
 * One poll for both: two components asking the same question on two timers
 * would be two answers that disagree for up to a minute - a badge saying "2"
 * above a list that has none new.
 *
 * Polled once a minute, which is the agent's own cadence (ADR 0014): a notice
 * is at most that late in the browser and in the toast alike. A hidden tab
 * asks nothing and asks again when it comes back, for the reason the live
 * column does (see `live.ts`).
 */

import { createContext, use, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { api, type Inbox } from '@/lib/api'

const POLL_SECONDS = 60

interface InboxState {
  /** `null` until the first answer, and after a failure with nothing to keep. */
  inbox: Inbox | null
  failed: boolean
  /** Asks now, outside the timer - after reading, so the badge follows. */
  refresh: () => void
  /** Moves the read cursor, then asks again. */
  markRead: (through: number) => Promise<void>
}

const InboxContext = createContext<InboxState | null>(null)

export function InboxProvider({ children }: { children: ReactNode }) {
  const [inbox, setInbox] = useState<Inbox | null>(null)
  const [failed, setFailed] = useState(false)
  // Held in refs rather than state: rescheduling is not a render, and the
  // poll itself lives inside the effect so that `refresh` can reach it
  // without the timer restarting on every tick (the shape `live.ts` uses).
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const pollNow = useRef<() => void>(() => {})

  useEffect(() => {
    let cancelled = false

    const schedule = () => {
      clearTimeout(timer.current)
      timer.current = setTimeout(poll, POLL_SECONDS * 1000)
    }

    const poll = () => {
      clearTimeout(timer.current)
      // Nobody looking: wait for the tab to come back.
      if (document.hidden) return
      api
        .notifications()
        .then((answer) => {
          if (cancelled) return
          setInbox(answer)
          setFailed(false)
        })
        .catch(() => {
          // The last answer stays. A dropped request is the network blinking,
          // and a badge that vanished would say "nothing new" - the reassuring
          // lie this product does not tell.
          if (!cancelled) setFailed(true)
        })
        .finally(() => {
          if (!cancelled) schedule()
        })
    }

    const onVisible = () => {
      if (!document.hidden) poll()
    }

    pollNow.current = poll
    poll()
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      cancelled = true
      clearTimeout(timer.current)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [])

  const poll = useCallback(() => pollNow.current(), [])

  const markRead = useCallback(
    async (through: number) => {
      await api.readNotifications(through)
      poll()
    },
    [poll],
  )

  const value = useMemo(() => ({ inbox, failed, refresh: poll, markRead }), [inbox, failed, poll, markRead])
  return <InboxContext value={value}>{children}</InboxContext>
}

export function useInbox(): InboxState {
  const inbox = use(InboxContext)
  if (!inbox) throw new Error('useInbox must be used inside <InboxProvider>')
  return inbox
}
