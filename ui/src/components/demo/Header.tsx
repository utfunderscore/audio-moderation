export function Header() {
  return (
    <header className="sticky top-0 z-30 border-b bg-background/80 backdrop-blur">
      <div className="flex h-14 w-full items-center gap-3 px-4 md:px-6">
        <span className="font-socialguard text-lg font-semibold tracking-normal">
          socialguard
        </span>
        <span className="h-4 w-px bg-border" aria-hidden />
        <span className="text-sm text-muted-foreground">Audio moderation</span>
      </div>
    </header>
  )
}
