import type { Metadata } from "next"
import { PageHero } from "@/components/page-hero"
import { PageNav } from "@/components/page-nav"
import { ArticlesSection } from "@/components/sections/articles"
import { loadArticles } from "@/lib/articles"
import { pageBySlug } from "@/lib/pages"

// The list is read at render time and rebuilt at most every five minutes.
export const revalidate = 300

const page = pageBySlug("/articles")

export const metadata: Metadata = { title: `${page.label} · Teeny's Mind`, description: page.lede }

export default async function Page() {
  const list = await loadArticles()
  return (
    <>
      <PageHero page={page} />
      <ArticlesSection list={list} />
      <PageNav slug={page.slug} />
    </>
  )
}
