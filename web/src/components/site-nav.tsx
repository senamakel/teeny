import Link from "next/link"
import { Brain, Code2, Menu } from "lucide-react"
import { Button } from "@/components/ui/button"
import {
  Sheet,
  SheetContent,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet"

const LINKS = [
  { href: "#reflections", label: "Reflections" },
  { href: "#soul", label: "Soul" },
  { href: "#rules", label: "Rules" },
  { href: "#tools", label: "Tools" },
  { href: "#memory", label: "Memory" },
  { href: "#lessons", label: "Lessons" },
]

export function SiteNav() {
  return (
    <header className="sticky top-0 z-40 border-b bg-background/80 backdrop-blur">
      <div className="mx-auto flex h-14 max-w-6xl items-center justify-between px-4 sm:px-6">
        <Link href="/" className="flex items-center gap-2 font-medium">
          <Brain className="size-5 text-primary" />
          <span>teeny</span>
          <span className="text-muted-foreground">/ mind</span>
        </Link>

        <nav className="hidden items-center gap-1 md:flex">
          {LINKS.map((link) => (
            <Button key={link.href} variant="ghost" size="sm" render={<a href={link.href} />}>
              {link.label}
            </Button>
          ))}
          <Button
            variant="outline"
            size="sm"
            className="ml-2"
            render={<a href="https://github.com/tinyhumansai" target="_blank" rel="noreferrer" />}
          >
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
              {LINKS.map((link) => (
                <Button
                  key={link.href}
                  variant="ghost"
                  className="justify-start"
                  render={<a href={link.href} />}
                >
                  {link.label}
                </Button>
              ))}
              <Button
                variant="outline"
                className="mt-4 justify-start"
                render={<a href="https://github.com/tinyhumansai" target="_blank" rel="noreferrer" />}
              >
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
