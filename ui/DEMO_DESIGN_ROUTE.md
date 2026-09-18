# Demo Page — Design Route (shadcn/ui + Untitled UI Icons)

Companion to `ui/DEMO_PLAN.md`. This document defines the visual direction, the
concrete shadcn component for every block, and the exact Untitled UI icon for every
glyph. It supersedes the "hand-rolled CSS" note in the plan.

> Revision: the page is now a vertical artifact timeline (Audio → Transcription
> → Moderation) that finishes at moderation, with a toast for the terminal
> outcome. Each stage page shows the artifact it produced. The event log, the
> Result page, and the side-by-side layout described below are parked; see
> `DEMO_PLAN.md` §0.

## 1. Direction

A dark, calm "control room": neutral shadcn surfaces, one accent color per pipeline
state (amber = processing, emerald = complete, red = failed, muted = pending), and
Untitled UI icons at 16/20px. No decoration competes with the stage tracker; the
stage cards and the event log are the only things that move.

- Component library: shadcn/ui, components copied into `src/components/ui/`.
- Icons: `@untitledui/icons` only. No Lucide glyphs in our source.
- Tailwind CSS v4 via `@tailwindcss/vite`; theme tokens in `src/index.css`.
- Dark mode by default (`<html class="dark">`), light mode inherits shadcn tokens.

## 2. Setup route

```sh
cd ui
npm create vite@latest . -- --template react-ts   # "Ignore files and continue"
npm install tailwindcss @tailwindcss/vite
npm install -D @types/node

# 1. Add the @/* alias in tsconfig.json and tsconfig.app.json, and the
#    tailwindcss plugin + alias in vite.config.ts (per shadcn Vite guide).
# 2. Replace src/index.css with `@import "tailwindcss";`
npx shadcn@latest init            # accept detected Vite; neutral base, CSS variables on

npx shadcn@latest add button card badge progress separator scroll-area select \
  switch tooltip alert dialog label tabs toggle-group slider skeleton sonner

npm install @untitledui/icons
```

Notes:

- Add only the components listed in §4; keep `src/components/ui/` minimal.
- If the init prompt offers an icon library, pick the default; generated files only
  use icons inside Select, Dialog, and Checkbox internals. Replace those imports
  with Untitled UI equivalents as they appear so `lucide-react` can be removed.
- Icons import by name for tree-shaking: `import { ShieldTick } from "@untitledui/icons"`.

## 3. Layout route

```
┌───────────────────────────────────────────────────────────────────────┐
│ Header (sticky): Activity mark · title · Simulation Badge · conn Badge│
├──────────────────────────────┬────────────────────────────────────────┤
│ lg: col-span-4               │ lg: col-span-8                         │
│ ┌ Audio panel (Card) ───────┐│ ┌ Run controls (Card) ───────────────┐ │
│ │ dropzone / file chip      ││ │ scenario Select · speed ToggleGroup│ │
│ │ <audio> + seek Slider     ││ │ late-subscribe Switch · Run/Restart│ │
│ │ waveform canvas           ││ └────────────────────────────────────┘ │
│ │ metadata · Run Button     ││ ┌ Stage tracker (Card) ──────────────┐ │
│ └───────────────────────────┘│ │ Submitted → Conversion → ASR →     │ │
│                              │ │ Moderation → Result                │ │
│                              │ │ (StageCards + animated connectors) │ │
│                              │ └────────────────────────────────────┘ │
│                              │ ┌ Terminal banner (Alert) ───────────┐ │
│                              │ └────────────────────────────────────┘ │
├──────────────────────────────┴────────────────────────────────────────┤
│ Event log (Card): header actions · ScrollArea rows · auto-scroll Switch│
└───────────────────────────────────────────────────────────────────────┘
```

- Desktop `lg+`: 12-column grid, audio panel left, pipeline right, event log full
  width below.
- `< lg`: single column. `Tabs` switch between **Pipeline** and **Events** so the
  log never pushes the stages off screen.
- Height discipline: event log `max-h-[320px]`, stage tracker natural height, header
  `h-14`.

## 4. Component map

| Block | shadcn components | Notes |
|---|---|---|
| Header | `Badge`, `Separator`, `Tooltip`, `Button` (variant ghost) | Simulation badge is `outline`; connection badge carries state dot; info button opens §8 dialog |
| Audio panel | `Card` (+Header/Title/Description/Content/Footer), `Button`, `Badge`, `Separator`, `Skeleton`, `Slider`, `Label` | Dropzone = styled `Label` wrapping hidden `input[type=file]`; file chip uses `Badge variant=secondary` |
| Waveform | — (custom `<canvas>`) inside `CardContent` | `Skeleton` while `decodeAudioData` runs; static bars if decode fails |
| Run controls | `Card`, `Select`, `ToggleGroup` (single), `Switch`, `Button`, `Label`, `Tooltip` | Speed = 1x / 4x / Instant; scenario Select items carry a one-line description |
| Stage tracker | `Card`, `Badge`, `Progress`, `Tooltip`, `Separator`, CVA variants | `StageCard` = `Card` with `data-status`; connectors are custom CSS, not `Separator` |
| Stage card internals | `Badge` (state), `Progress` (active only), `Tooltip` (elapsed) | Processing uses an animated indeterminate `Progress` (`indeterminate` prop is not native; overlay a CSS gradient) |
| Event log | `Card`, `ScrollArea`, `Badge`, `Button` (ghost, icon), `Separator`, `Switch`, `Skeleton`, `Kbd` | Rows are plain `<ol>` children; badges flag `replay` / `duplicate`; copy button per row |
| Terminal banner | `Alert` + `AlertTitle` + `AlertDescription`, `Badge`, `Button` | Variant by outcome; CTA = Restart, secondary = open event log |
| Info dialog | `Dialog` (+Trigger/Content/Header/Title/Description/Footer) | Explains simulation + lists the 8 event names |
| Mobile tabs | `Tabs` (+List/Trigger/Content) | Pipeline / Events |
| Completion toast | `Sonner` | One toast on terminal outcome; do not toast every event |

Do not add `Table` (event rows are not tabular), `Accordion`, `Sheet`, or `Drawer`
for this page.

## 5. Icon map (@untitledui/icons, exact names)

| Where | Icon | Import name |
|---|---|---|
| Header mark | activity pulse | `Activity` |
| Simulation badge | info | `InfoCircle` |
| Connection: subscribed | signal | `Signal01` |
| Connection: connecting | spinner (animate-spin) | `Loading01` |
| Connection: closed / cancelled | slash | `SlashCircle01` |
| Connection: error | alert | `AlertCircle` |
| Audio dropzone | upload cloud | `UploadCloud02` |
| Sample audio button | headphones | `Headphones01` |
| Playback play / pause | play, pause | `Play`, `PauseCircle` |
| Remove audio | trash | `Trash01` |
| Stage: Submitted | file check | `FileCheck02` |
| Stage: Conversion | scissors | `Scissors01` |
| Stage: Transcription | microphone | `Microphone01` |
| Stage: Moderation | shield check | `ShieldTick` |
| Stage: Result | check circle / alert | `CheckCircle`, `AlertTriangle` |
| Stage state: pending | clock | `Clock` |
| Stage state: processing | spinner (animate-spin) | `Loading01` |
| Stage state: complete | none — the label already says "Complete" | — |
| Stage state: failed | alert circle | `AlertCircle` |
| Stage state: skipped | slash | `SlashCircle01` |
| Event log header | terminal | `Terminal` |
| Event row: replay | repeat | `Repeat01` |
| Event row: duplicate | copy | `Copy01` |
| Event row: copy | copy | `Copy01` |
| Auto-scroll toggle | arrow down | `ArrowDown` |
| Clear log | trash | `Trash01` |
| Scenario select | sliders | `Sliders01` |
| Speed toggle | zap | `Zap` |
| Late subscribe | repeat | `Repeat01` |
| Run / Restart | play / refresh | `Play`, `RefreshCw01` |
| Outcome: succeeded | check circle | `CheckCircle` |
| Outcome: failed | alert triangle | `AlertTriangle` |
| Outcome: timed out | clock rewind | `ClockRewind` |
| Outcome: cancelled | slash | `SlashCircle01` |

Rules: `size-4` (16px) inline, `size-5` (20px) for stage-card headers, `size-6` for
the empty-state dropzone. Icons next to text are `aria-hidden`; icon-only buttons get
an `aria-label` and a `Tooltip`. Spinners use `animate-spin` and respect
`motion-reduce:animate-none`.

## 6. Status tokens

Add to `src/index.css` and map through `@theme inline` so utilities like
`bg-success/10` and `text-warning` exist:

```css
:root, .dark {
  --success: oklch(0.72 0.15 162);   /* emerald-ish */
  --success-foreground: oklch(0.98 0.01 162);
  --warning: oklch(0.80 0.15 78);    /* amber-ish */
  --warning-foreground: oklch(0.27 0.05 78);
}
@theme inline {
  --color-success: var(--success);
  --color-success-foreground: var(--success-foreground);
  --color-warning: var(--warning);
  --color-warning-foreground: var(--warning-foreground);
}
```

| Stage state | Classes |
|---|---|
| pending | `border-border text-muted-foreground` |
| processing | `border-warning/40 bg-warning/5 text-warning` |
| complete | `border-success/40 bg-success/5 text-success` |
| failed | `border-destructive/40 bg-destructive/5 text-destructive` |
| skipped | `border-border/60 text-muted-foreground/60` |

Terminal `Alert` uses the same tokens: success → `text-success`, failed →
`destructive`, timed out/cancelled → `text-muted-foreground`.

## 7. Screen composition

**Header** — `Activity` + "SocialGuard · Audio Moderation Demo" (`text-sm
font-semibold`), right side: `Badge variant=outline` "Simulation" with `InfoCircle`
tooltip, `Badge` connection state with state dot, ghost `InfoCircle` button opening
the dialog.

**Audio panel** — when empty: dashed dropzone (`border-dashed border-2 rounded-lg
p-8`) with `UploadCloud02 size-6`, primary "Choose audio" `Button`, and a
`Button variant=link` "Use sample" with `Headphones01`. When loaded: filename +
size `Badge`s, `<audio>` element, seek `Slider`, waveform canvas, metadata row
(duration / format / sample rate / channels) separated by `Separator`, and a
`Button variant=ghost size=icon` with `Trash01` to clear.

**Stage tracker** — horizontal row on `lg` with CSS connectors that fill with the
success color when the downstream stage activates. Each `StageCard`:
icon + name (`CardHeader`), state `Badge`, one-line description, elapsed time, and
an indeterminate `Progress` only while processing. Result card headline changes by
outcome. `aria-live="polite"` announces transitions ("Transcription started",
"Moderation complete", …).

**Event log** — `CardHeader` with `Terminal`, event count `Badge`, autoscroll
`Switch` + `ArrowDown`, clear `Button` + `Trash01`. Rows: monospace timestamp,
event name, right-aligned `Badge`s (`Repeat01` replay, `Copy01` duplicate), copy
`Button`. Auto-scroll pauses when the user scrolls up. Terminal rows use the status
tokens.

**Run controls** — scenario `Select` (9 fixtures from the plan §6), speed
`ToggleGroup`, late-subscribe `Switch` with `Repeat01`, then `Button` "Run
evaluation" (`Play`) / "Restart" (`RefreshCw01`). Run is disabled while a scenario
is active; Restart is disabled before the first run.

**Info dialog** — three short sections: "This is a simulation", "What the WebSocket
sends" (raw event-name frames, replay-then-live, at-least-once), "The 8 lifecycle
events" (monospace chips). Footer button "Got it".

## 8. Interaction notes

- Only one shadcn `TooltipProvider` at the app root.
- `Select` is controlled; changing the scenario mid-run requires Restart (disable
  the Select while running, show the reason in its `Tooltip`).
- `ToggleGroup` and `Switch` follow shadcn controlled patterns; speed changes apply
  to the mock client without restarting the run.
- `ScrollArea` needs an explicit height to scroll; the event log owns it.
- `Sonner` gets a single `Toaster` and only terminal outcomes raise toasts.

## 9. Accessibility & motion

- Focus ring uses shadcn `--ring`; never remove outlines.
- Icon-only controls: `aria-label` + `Tooltip`.
- Stage transitions announced through one polite live region, not per card.
- Event log is an ordered list; new rows do not steal focus.
- `motion-reduce:animate-none` on spinners and connectors; no auto-playing
  animation except the processing state.

## 10. Build order

1. Foundations: Vite, Tailwind v4, shadcn init, tokens §6, `@untitledui/icons`.
2. `Button` / `Card` / `Badge` layout skeleton with static stage content.
3. `StageCard` + tracker connectors + `Progress`.
4. Audio panel with waveform and `Slider`.
5. Event log with `ScrollArea` and badges.
6. Run controls + `Dialog` + `Sonner`.
7. Mobile `Tabs`, a11y pass, motion pass.

## 11. Acceptance checklist

- [ ] No `lucide-react` imports remain in `src/`; all icons come from `@untitledui/icons`.
- [ ] Every stage state in §6 is visually distinct in both dark and light mode.
- [ ] Event log scrolls independently and pauses auto-scroll on user scroll.
- [ ] Tab order: audio → controls → stages → log; icon buttons labelled.
- [ ] Page reads at 1280px and 390px wide without clipping the stage strip.
