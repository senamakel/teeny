import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { Lessons } from "@/components/sections/lessons"
import { pageBySlug } from "@/lib/pages"

const page = pageBySlug("/lessons")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default function Page() {
  return (
    <>
      <PageHero page={page} />
      <Lessons />
      <PageNav slug={page.slug} />
    </>
  )
}
