import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { Tools } from "@/components/sections/tools"
import { pageBySlug } from "@/lib/pages"

const page = pageBySlug("/tools")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default function Page() {
  return (
    <>
      <PageHero page={page} />
      <Tools />
      <PageNav slug={page.slug} />
    </>
  )
}
