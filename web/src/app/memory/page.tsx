import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { Memory } from "@/components/sections/memory"
import { pageBySlug } from "@/lib/pages"

const page = pageBySlug("/memory")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default function Page() {
  return (
    <>
      <PageHero page={page} />
      <Memory />
      <PageNav slug={page.slug} />
    </>
  )
}
