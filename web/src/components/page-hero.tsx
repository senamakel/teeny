import { SceneCanvas } from '@/components/scene'
import type { PageMeta } from '@/lib/pages'

export function PageHero({ page }: { page: PageMeta }) {
  return (
    <section className="relative overflow-hidden border-b">
      <SceneCanvas name={page.scene} className="absolute inset-y-0 right-0 h-full w-full lg:w-3/5" />
      {/* Keeps the text legible over whatever the scene is doing. */}
      <div className="pointer-events-none absolute inset-0 bg-linear-to-r from-background via-background/60 to-transparent lg:via-background/30" />
      <div className="pointer-events-none absolute inset-0 bg-linear-to-t from-background via-background/60 to-transparent lg:hidden" />
      <div className="relative mx-auto flex min-h-[52vh] max-w-6xl flex-col justify-end px-4 pt-24 pb-14 sm:px-6 sm:pb-20">
        <p className="mb-3 font-mono text-xs uppercase tracking-widest text-primary">{page.eyebrow}</p>
        <h1 className="max-w-3xl text-4xl font-semibold tracking-tight sm:text-5xl">{page.title}</h1>
        <p className="mt-4 max-w-2xl text-lg text-muted-foreground">{page.lede}</p>
      </div>
    </section>
  )
}
