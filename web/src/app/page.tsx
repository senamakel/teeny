import { SiteFooter } from "@/components/site-footer"
import { SiteNav } from "@/components/site-nav"
import { Hero } from "@/components/sections/hero"
import { Lessons } from "@/components/sections/lessons"
import { Memory } from "@/components/sections/memory"
import { Reflections } from "@/components/sections/reflections"
import { Rules } from "@/components/sections/rules"
import { Soul } from "@/components/sections/soul"
import { Tools } from "@/components/sections/tools"
import { loadMind } from "@/lib/mind"

export default async function Home() {
  const feed = await loadMind()
  return (
    <>
      <SiteNav />
      <main className="flex-1">
        <Hero feed={feed} />
        <Reflections feed={feed} />
        <Soul />
        <Rules />
        <Tools />
        <Memory />
        <Lessons />
      </main>
      <SiteFooter />
    </>
  )
}
