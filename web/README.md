# teeny / mind

The landing page for Teeny's mind: its soul, the rules it runs under, the tools
it can reach, how its memory works, a log of reflections behind each reply, and
the mistakes it made in public.

Next.js 16, Tailwind v4, shadcn/ui (Base UI primitives), three.js via
react-three-fiber.

## Pages

One route per part of the mind, each with its own scene behind the hero:

| Route | Scene |
| --- | --- |
| `/` | a brain: points on a sphere, linked to their neighbours, neurons firing |
| `/reflections` | a stream of thoughts flowing past a few that settled |
| `/soul` | one warm, soft, restless blob |
| `/rules` | a core held inside a cage of turning rings |
| `/tools` | five rings of tools orbiting a core, one ring per group |
| `/memory` | a lattice of points; a recall lights a cluster and fades |
| `/lessons` | a sphere that keeps coming apart and mending |

`src/lib/pages.ts` is the registry: nav, home cards, heroes and prev/next all
read from it. Scenes live in `src/components/scene/scenes/` and honour
`prefers-reduced-motion` by standing still.

```sh
npm install
npm run dev      # http://localhost:3000
npm run build
```

## The feed

`src/lib/mind.ts` defines the `MindFeed` shape the page renders. Set
`MIND_FEED_URL` to a JSON endpoint serving that shape and the page reads it
(revalidated every 60s) and marks itself live. Unset, it shows sample entries
reconstructed from incidents documented in the bot's code and history, and says
so on the page.

The bot does not publish this feed yet. Wiring it means recording, per turn:
the incoming text, the tone the drafter read, the recall summaries, the tool
calls, the reasoning, the reply, what it declined, and the outcome. Everything
static on the page (`src/lib/brain.ts`) is sourced from `SOUL.md`, `CONTEXT.md`,
`README.md` and the published tool list.
