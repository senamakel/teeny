import { loadTotals } from "@/lib/ledger"

/**
 * What the counter on the home page polls: the running token count, read
 * from the bot on every request. Small, uncached, and the only thing the
 * browser asks the page for after it has loaded.
 */
export const dynamic = "force-dynamic"

export async function GET() {
  const { source, totals } = await loadTotals()
  return Response.json(
    { source, totals, at: new Date().toISOString() },
    { headers: { "cache-control": "no-store" } },
  )
}
