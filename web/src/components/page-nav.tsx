import Link from "next/link"
import { ArrowLeft, ArrowRight } from "lucide-react"
import { Button } from "@/components/ui/button"
import { neighbours } from "@/lib/pages"

export function PageNav({ slug }: { slug: string }) {
  const { prev, next } = neighbours(slug)
  return (
    <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 border-t px-4 py-8 sm:px-6">
      {prev ? (
        <Button variant="ghost" render={<Link href={prev.slug} />}>
          <ArrowLeft className="size-4" />
          {prev.label}
        </Button>
      ) : (
        <Button variant="ghost" render={<Link href="/" />}>
          <ArrowLeft className="size-4" />
          Home
        </Button>
      )}
      {next ? (
        <Button variant="ghost" render={<Link href={next.slug} />}>
          {next.label}
          <ArrowRight className="size-4" />
        </Button>
      ) : (
        <Button variant="ghost" render={<Link href="/" />}>
          Home
          <ArrowRight className="size-4" />
        </Button>
      )}
    </div>
  )
}
