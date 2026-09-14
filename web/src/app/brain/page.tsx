import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { BrainSection } from "@/components/sections/brain"
import { loadBrain, loadRepo } from "@/lib/company"
import { pageBySlug } from "@/lib/pages"

// The snapshot is read at render time and the page is rebuilt at most once a minute.
export const revalidate = 60

const page = pageBySlug("/brain")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default async function Page() {
  const [snapshot, repo] = await Promise.all([loadBrain(), loadRepo()])
  return (
    <>
      <PageHero page={page} />
      <BrainSection snapshot={snapshot} repo={repo} />
      <PageNav slug={page.slug} />
    </>
  )
}
