import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { Ledger } from "@/components/sections/ledger"
import { loadLedger } from "@/lib/ledger"
import { loadMind } from "@/lib/mind"
import { pageBySlug } from "@/lib/pages"

// Both feeds are read at render time and the page is rebuilt at most once a minute.
export const revalidate = 60

const page = pageBySlug("/ledger")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default async function Page() {
  const [ledger, mind] = await Promise.all([loadLedger({ limit: 80 }), loadMind()])
  return (
    <>
      <PageHero page={page} />
      <Ledger ledger={ledger} mind={mind} />
      <PageNav slug={page.slug} />
    </>
  )
}
