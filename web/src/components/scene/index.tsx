'use client'

import dynamic from 'next/dynamic'
import type { SceneName } from '@/lib/pages'

const Scene = dynamic(() => import('./scene').then((m) => m.Scene), {
  ssr: false,
  loading: () => null,
})

export function SceneCanvas({ name, className }: { name: SceneName; className?: string }) {
  return (
    <div className={className} aria-hidden>
      <Scene name={name} />
    </div>
  )
}
