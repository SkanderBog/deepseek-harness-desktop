import { Navbar } from './navbar'

export function Webview() {
  return (
    <main className="relative flex flex-col min-h-0 flex-1">
      <Navbar />
      <div className="relative flex min-h-0 flex-1" data-testid="dsh-shell-content" />
    </main>
  )
}
