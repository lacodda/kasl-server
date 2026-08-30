/**
 * The live feed behind the dashboard's status column.
 *
 * The server holds one pulse per agent and decides what it means; this side
 * only asks again on the cadence the server names. Two rules keep the polling
 * from becoming a nuisance:
 *
 *   * **A hidden tab asks nothing.** A dashboard left open on a second monitor
 *     overnight would otherwise poll a thousand times before anybody looked at
 *     it. Asking again the moment the tab comes back is what a person actually
 *     wants: fresh figures when they look, silence when they do not.
 *   * **A failed poll keeps the last answer.** A dropped request means the
 *     network blinked, not that everyone stopped working, and blanking the
 *     column would say the second.
 */

import { useEffect, useRef, useState } from 'react'
import { api, type LiveMember, type LiveStatus } from '@/lib/api'

/** How long to wait before asking again when the server could not be reached. */
const RETRY_SECONDS = 30

export interface LiveFeed {
  /** Status by user id, empty until the first answer arrives. */
  byUser: Map<string, LiveMember>
  /** Whether an answer has ever arrived; until it has, the column says nothing. */
  loaded: boolean
}

/**
 * Polls `/team/live` for as long as the component is mounted and the tab is
 * visible.
 *
 * `enabled` is false for a reader who may not call the endpoint at all - an
 * employee on their own page - so the hook can sit in a shared component
 * without polling for a 403 forever.
 */
export function useLiveTeam(enabled = true): LiveFeed {
  const [feed, setFeed] = useState<LiveFeed>({ byUser: new Map(), loaded: false })
  // Held in a ref rather than state: rescheduling is not a render, and putting
  // the timer in the dependency list would restart the poll on every tick.
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)

  useEffect(() => {
    if (!enabled) return
    let cancelled = false

    const schedule = (seconds: number) => {
      clearTimeout(timer.current)
      timer.current = setTimeout(poll, seconds * 1000)
    }

    const poll = () => {
      // Nothing to show and nobody looking: wait for the tab to come back
      // rather than burn a request into a page nobody is reading.
      if (document.hidden) return

      api
        .teamLive()
        .then((answer) => {
          if (cancelled) return
          setFeed({ byUser: new Map(answer.members.map((member) => [member.user_id, member])), loaded: true })
          schedule(answer.poll_seconds)
        })
        .catch(() => {
          if (cancelled) return
          // The previous answer stays on screen. It ages, which the row shows
          // by falling back to "offline" once the server says so - a lie of
          // omission is better than a blank column that reads as "nobody".
          schedule(RETRY_SECONDS)
        })
    }

    const onVisible = () => {
      if (!document.hidden) poll()
    }

    poll()
    document.addEventListener('visibilitychange', onVisible)

    return () => {
      cancelled = true
      clearTimeout(timer.current)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [enabled])

  return feed
}

/**
 * How a status should be drawn: the colour role and whether it counts as
 * someone being at work right now.
 *
 * Kept out of the component so the mapping can be asserted directly - a status
 * silently falling through to a default colour is exactly the kind of defect
 * that survives a screenshot.
 */
export function statusTone(status: LiveStatus): { tone: 'good' | 'warn' | 'dim' | 'faint'; live: boolean } {
  switch (status) {
    case 'working':
      return { tone: 'good', live: true }
    case 'paused':
      return { tone: 'warn', live: true }
    case 'idle':
      // Their agent is up and reporting; they are simply not in a day. That is
      // a working installation, not a problem, so it is not painted as one.
      return { tone: 'dim', live: false }
    case 'offline':
      return { tone: 'dim', live: false }
    case 'unknown':
      // No pulse ever. Faint rather than warned about: on a team still rolling
      // kasl out this is most of the table, and a page of warnings about
      // agents too old to send a pulse teaches people to ignore the column.
      return { tone: 'faint', live: false }
  }
}
