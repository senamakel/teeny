import type { ReactNode } from "react"

export function Section({
  id,
  eyebrow,
  title,
  lede,
  children,
}: {
  id: string
  eyebrow: string
  title: string
  lede?: string
  children: ReactNode
}) {
  return (
    <section id={id} className="scroll-mt-20 py-16 sm:py-20">
      <div className="mx-auto max-w-6xl px-4 sm:px-6">
        <div className="mb-8 max-w-2xl">
          <p className="mb-2 font-mono text-xs uppercase tracking-widest text-primary">{eyebrow}</p>
          <h2 className="text-2xl font-semibold tracking-tight sm:text-3xl">{title}</h2>
          {lede ? <p className="mt-3 text-muted-foreground">{lede}</p> : null}
        </div>
        {children}
      </div>
    </section>
  )
}
