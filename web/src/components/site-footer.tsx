export function SiteFooter() {
  return (
    <footer className="border-t">
      <div className="mx-auto flex max-w-6xl flex-col gap-3 px-4 py-8 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between sm:px-6">
        <p>
          Teeny is a community mascot. It never promotes another token, never takes a wallet
          action, and never predicts a price. Nothing here is financial advice.
        </p>
        <p className="font-mono text-xs">
          <a href="https://github.com/tinyhumansai" className="hover:text-foreground">github.com/tinyhumansai</a>
        </p>
      </div>
    </footer>
  )
}
