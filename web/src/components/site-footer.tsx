import { PresenceIcon } from "@/components/icons/brand"
import { PRESENCE } from "@/lib/presence"

export function SiteFooter() {
  return (
    <footer className="border-t">
      <div className="mx-auto flex max-w-6xl flex-col gap-5 px-4 py-8 text-sm text-muted-foreground sm:px-6">
        <ul className="flex flex-wrap items-center gap-x-5 gap-y-2">
          {PRESENCE.map((p) => (
            <li key={p.kind}>
              <a
                href={p.href}
                target="_blank"
                rel="noreferrer"
                className="flex items-center gap-2 transition-colors hover:text-foreground"
              >
                <PresenceIcon kind={p.kind} className="size-4" />
                <span className="font-mono text-xs">{p.handle}</span>
              </a>
            </li>
          ))}
        </ul>
        <p>
          Teeny is a community mascot. It never promotes another token, never takes a wallet
          action, and never predicts a price. Nothing here is financial advice.
        </p>
      </div>
    </footer>
  )
}
