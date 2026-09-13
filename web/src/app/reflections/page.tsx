import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { Reflections } from "@/components/sections/reflections"
import { pageBySlug } from "@/lib/pages"
import { loadMind } from "@/lib/mind"

// The feed is read at render time and the page is rebuilt at most once a minute.
export const revalidate = 60

const page = pageBySlug("/reflections")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default async function Page() {
  const feed = await loadMind()
  return (
    <>
      <PageHero page={page} />
      <Reflections feed={feed} />
      <PageNav slug={page.slug} />
    </>
  )
}
