import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { Soul } from "@/components/sections/soul"
import { pageBySlug } from "@/lib/pages"

const page = pageBySlug("/soul")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default function Page() {
  return (
    <>
      <PageHero page={page} />
      <Soul />
      <PageNav slug={page.slug} />
    </>
  )
}
