import { Hero } from "@/components/sections/hero"
import { HomeBrain } from "@/components/sections/home-brain"
import { HomeLedger } from "@/components/sections/home-ledger"
import { Overview } from "@/components/sections/overview"
import { PresenceSection } from "@/components/sections/presence"
import { loadBrain, loadRepo } from "@/lib/company"
import { loadLedger } from "@/lib/ledger"
import { loadMind } from "@/lib/mind"

// The feeds are read at render time and the page is rebuilt at most once a
// minute; the counter itself keeps polling from the browser.
export const revalidate = 60

export default async function Home() {
  const [feed, ledger, brain, repo] = await Promise.all([loadMind(), loadLedger({ limit: 8 }), loadBrain(), loadRepo()])
  return (
    <>
      <Hero feed={feed} />
      <HomeBrain snapshot={brain} repo={repo} />
      <HomeLedger ledger={ledger} />
      <PresenceSection />
      <Overview />
    </>
  )
}
