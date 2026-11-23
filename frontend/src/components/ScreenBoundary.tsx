import { Component, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'

/**
 * Catches a screen that failed to arrive, so the page around it stays.
 *
 * The screens are fetched when they are first opened (App.tsx), and a fetch
 * can fail: the connection dropped, or the server was upgraded while the page
 * stood open and its new build no longer has the old screen's file. Without a
 * boundary React unmounts the whole tree for it, and the reader is left with
 * a blank page and no way back but the address bar. With one, the header and
 * the tabs stay, and the screen's place says what happened.
 *
 * Reloading is the one way out offered, because it is the one that works in
 * both cases: it fetches the page the server has now, and the screen with it.
 * It waits for the reader's press rather than happening on its own: while the
 * connection is down, a page that reloads itself on a failed fetch does so in
 * a loop. A screen that throws while drawing lands here too, and the same
 * words are true of it.
 */
export class ScreenBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false }

  static getDerivedStateFromError() {
    return { failed: true }
  }

  render() {
    return this.state.failed ? <ScreenFailed /> : this.props.children
  }
}

function ScreenFailed() {
  const { t } = useTranslation()
  return (
    <div role="alert" className="mx-auto flex max-w-sm flex-col items-center gap-3 py-12 text-center">
      <p className="text-base font-semibold">{t('screen.failed')}</p>
      <p className="text-sm text-dim">{t('screen.failedDetail')}</p>
      <Button onClick={() => window.location.reload()}>{t('screen.reload')}</Button>
    </div>
  )
}
