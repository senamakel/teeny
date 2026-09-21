import { mediaSource } from "@/lib/articles"

// The bot is on the compose network and a browser is not, so a picture is
// fetched here and passed on, with the bot's content type and a day of
// cache. The slug and the file name are checked before any address is built.
export const dynamic = "force-dynamic"

export async function GET(_request: Request, { params }: { params: Promise<{ slug: string; file: string }> }) {
  const { slug, file } = await params
  const source = mediaSource(slug, file)
  if (!source) return new Response("not found", { status: 404 })
  try {
    const upstream = await fetch(source, { signal: AbortSignal.timeout(10_000), cache: "no-store" })
    if (!upstream.ok || !upstream.body) return new Response("not found", { status: 404 })
    return new Response(upstream.body, {
      status: 200,
      headers: {
        "content-type": upstream.headers.get("content-type") ?? "application/octet-stream",
        "cache-control": "public, max-age=86400",
      },
    })
  } catch {
    return new Response("unavailable", { status: 503 })
  }
}
