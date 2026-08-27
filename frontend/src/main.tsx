import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router'
import { App } from '@/App'
import { SessionProvider } from '@/lib/session'
import '@/i18n'
import '@/styles.css'

const root = document.getElementById('root')
if (!root) throw new Error('the #root element is missing from index.html')

createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <SessionProvider>
        <App />
      </SessionProvider>
    </BrowserRouter>
  </StrictMode>,
)
