'use client'

import Link from "next/link"
import { usePathname } from "next/navigation"
import { Brain, Code2, Menu } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Sheet, SheetContent, SheetTitle, SheetTrigger } from "@/components/ui/sheet"
import { PAGES } from "@/lib/pages"
import { cn } from "@/lib/utils"

const SOURCE = "https://github.com/tinyhumansai"

export function SiteNav() {
  const pathname = usePathname()
  return (
    <header className="fixed inset-x-0 top-0 z-40 border-b border-border/60 bg-background/70 backdrop-blur">
      <div className="mx-auto flex h-14 max-w-6xl items-center justify-between px-4 sm:px-6">
        <Link href="/" className="flex items-center gap-2 font-medium">
          <Brain className="size-5 text-primary" />
          <span>teeny</span>
          <span className="text-muted-foreground">/ mind</span>
        </Link>

        <nav className="hidden items-center gap-1 md:flex">
          {PAGES.map((page) => (
            <Button
              key={page.slug}
              variant="ghost"
              size="sm"
              className={cn(pathname === page.slug && "bg-muted text-foreground")}
              render={<Link href={page.slug} />}
            >
              {page.label}
            </Button>
          ))}
          <Button variant="outline" size="sm" className="ml-2" render={<a href={SOURCE} target="_blank" rel="noreferrer" />}>
            <Code2 className="size-4" />
            Source
          </Button>
        </nav>

        <Sheet>
          <SheetTrigger render={<Button variant="ghost" size="icon" className="md:hidden" />}>
            <Menu className="size-5" />
            <span className="sr-only">Open menu</span>
          </SheetTrigger>
          <SheetContent side="right" className="w-64">
            <SheetTitle className="sr-only">Navigation</SheetTitle>
            <nav className="mt-8 flex flex-col gap-1 px-4">
              {PAGES.map((page) => (
                <Button
                  key={page.slug}
                  variant="ghost"
                  className={cn("justify-start", pathname === page.slug && "bg-muted")}
                  render={<Link href={page.slug} />}
                >
                  {page.label}
                </Button>
              ))}
              <Button variant="outline" className="mt-4 justify-start" render={<a href={SOURCE} target="_blank" rel="noreferrer" />}>
                <Code2 className="size-4" />
                Source
              </Button>
            </nav>
          </SheetContent>
        </Sheet>
      </div>
    </header>
  )
}
