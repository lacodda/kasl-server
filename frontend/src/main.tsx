import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router'
import { overlayScrollbars } from 'dowel-ui'
import { App } from '@/App'
import { SessionProvider } from '@/lib/session'
import '@/i18n'
import '@/styles.css'

const root = document.getElementById('root')
if (!root) throw new Error('the #root element is missing from index.html')

// Before the first paint, so no screen draws with one kind of bar and then
// another. Where the system draws bars that take no width - a phone, a Mac -
// the page gets those instead of the theme's strip; on Windows a browser
// cannot ask for that, and the strip stays.
overlayScrollbars()

createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <SessionProvider>
        <App />
      </SessionProvider>
    </BrowserRouter>
  </StrictMode>,
)
