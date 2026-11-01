import { OverlaysProvider } from '@overlastic/react'
import { QueryClientProvider } from '@tanstack/react-query'
import React from 'react'
import ReactDOM from 'react-dom/client'
import { ToastProvider } from './components/toast-provider'
import { queryClient } from './config/client'
import { resolveTheme } from './hooks/use-theme-adaptive'
import { App } from './layout'
import '@/utils/logger'
import './styles/main.css'

document.documentElement.dataset.theme = resolveTheme(
  undefined,
  window.matchMedia('(prefers-color-scheme: dark)').matches,
)

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <OverlaysProvider>
          <App />
        </OverlaysProvider>
      </ToastProvider>
    </QueryClientProvider>
  </React.StrictMode>,
)
