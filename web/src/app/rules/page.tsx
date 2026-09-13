import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { Rules } from "@/components/sections/rules"
import { pageBySlug } from "@/lib/pages"

const page = pageBySlug("/rules")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default function Page() {
  return (
    <>
      <PageHero page={page} />
      <Rules />
      <PageNav slug={page.slug} />
    </>
  )
}
