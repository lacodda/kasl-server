import { useEffect, useState } from 'react'
import { Link } from 'react-router'
import { useTranslation } from 'react-i18next'
import type { Notification } from '@/lib/api'
import { moment } from '@/lib/day'
import { useInbox } from '@/lib/inbox'
import { noticeState, readThrough, screenFor, segments } from '@/lib/notifications'
import { Panel } from '@/components/ui/panel'

/**
 * What the server has told you.
 *
 * The same notices kasl shows as toasts on your machines, kept so they can be
 * read again (ADR 0020). Opening the screen is reading it: once the list is on
 * screen the cursor moves to the newest notice shown, the badge clears, and
 * kasl stops toasting what was read here.
 *
 * Reached from the bell in the header rather than from the navigation: the
 * phone's bar holds five screens and has five, and the bell is where people
 * look for exactly this.
 */
export function Notifications() {
  const { t } = useTranslation()
  const { inbox, failed, markRead } = useInbox()
  // The cursor as it stood when this screen opened, taken from the first
  // answer and then held: what was new on arrival stays marked for the visit,
  // though the server has been told it is read (see `noticeState`).
  const [openedAt, setOpenedAt] = useState<number | null>(null)
  if (inbox && openedAt === null) setOpenedAt(inbox.read_through)

  // After the list is drawn, not before: what is marked read has to be what
  // the reader has in front of them. `readThrough` answers null when nothing
  // on screen is new, so reopening the inbox posts nothing.
  const through = inbox ? readThrough(inbox.notifications, inbox.read_through) : null
  useEffect(() => {
    if (through !== null) void markRead(through).catch(() => {})
  }, [through, markRead])

  if (!inbox) {
    return <p className={`text-sm ${failed ? 'text-bad' : 'text-dim'}`}>{t(failed ? 'common.error' : 'common.loading')}</p>
  }

  return (
    <div className="mx-auto max-w-3xl space-y-5">
      <div>
        <h1 className="text-lg font-semibold">{t('notifications.title')}</h1>
        <p className="mt-2 text-sm text-dim">{t('notifications.subtitle')}</p>
      </div>

      {inbox.notifications.length === 0 ? (
        <Panel className="p-4 sm:p-5">
          <p className="text-sm text-dim">{t('notifications.none')}</p>
        </Panel>
      ) : (
        <Panel className="divide-y divide-line">
          {inbox.notifications.map((notice) => (
            <Notice key={notice.id} notice={notice} openedAt={openedAt ?? inbox.read_through} />
          ))}
        </Panel>
      )}
    </div>
  )
}

function Notice({ notice, openedAt }: { notice: Notification; openedAt: number }) {
  const { t } = useTranslation()
  const state = noticeState(notice, openedAt)
  const screen = screenFor(notice)

  return (
    <article className="flex gap-3 px-4 py-3.5 sm:px-5" aria-label={notice.title}>
      {/* The mark for a new notice. A dot rather than a colour on the text: the
          title keeps one colour in both themes and every accent, and the dot
          is the only thing that changes when it is read. */}
      <span
        aria-hidden
        className={`mt-1.5 size-2 shrink-0 rounded-full ${state === 'new' ? 'bg-accent' : 'bg-transparent'}`}
      />
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex flex-col gap-0.5 sm:flex-row sm:items-baseline sm:justify-between sm:gap-3">
          <h2 className={`text-sm ${state === 'over' ? 'text-dim' : 'font-medium'}`}>
            {state === 'new' && <span className="sr-only">{t('notifications.unread')}: </span>}
            {notice.title}
          </h2>
          <span className="shrink-0 font-mono text-xs text-faint tabular">{moment(notice.created_at)}</span>
        </div>
        <p className={`break-words text-sm ${state === 'over' ? 'text-faint' : 'text-dim'}`}>
          {segments(notice.body).map((segment, index) =>
            segment.code ? (
              // Never broken inside: `2026-` on one line and `09-24` on the
              // next is a command nobody can paste. On a screen too narrow
              // for it, it scrolls within itself instead.
              <code
                key={index}
                className="inline-block max-w-full overflow-x-auto whitespace-nowrap rounded-inner bg-soft px-1 py-0.5 align-middle font-mono text-xs text-text"
              >
                {segment.text}
              </code>
            ) : (
              <span key={index}>{segment.text}</span>
            ),
          )}
        </p>
        {(state === 'over' || screen) && (
          <p className="flex flex-wrap gap-x-3 text-xs">
            {/* Said rather than hidden: "your manager was told on Monday"
                stays a fact after Tuesday fixed it, and the reader should
                see that it was fixed. */}
            {notice.withdrawn_at && <span className="text-faint">{t('notifications.over', { at: moment(notice.withdrawn_at) })}</span>}
            {screen && (
              <Link to={screen} className="text-accent-2 underline-offset-2 hover:underline">
                {t(screen === '/day' ? 'notifications.openDay' : 'notifications.openPrivacy')}
              </Link>
            )}
          </p>
        )}
      </div>
    </article>
  )
}
