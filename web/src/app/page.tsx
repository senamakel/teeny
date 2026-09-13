import { Hero } from "@/components/sections/hero"
import { Overview } from "@/components/sections/overview"
import { loadMind } from "@/lib/mind"

export default async function Home() {
  const feed = await loadMind()
  return (
    <>
      <Hero feed={feed} />
      <Overview />
    </>
  )
}
