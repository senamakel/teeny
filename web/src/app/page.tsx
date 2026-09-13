import { Hero } from "@/components/sections/hero"
import { Overview } from "@/components/sections/overview"
import { loadMind } from "@/lib/mind"

// The feed is read at render time and the page is rebuilt at most once a minute.
export const revalidate = 60

export default async function Home() {
  const feed = await loadMind()
  return (
    <>
      <Hero feed={feed} />
      <Overview />
    </>
  )
}
