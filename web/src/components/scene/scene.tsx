'use client'

import { Canvas } from '@react-three/fiber'
import type { SceneName } from '@/lib/pages'
import { BrainScene } from './scenes/brain'
import { GridScene } from './scenes/grid'
import { LatticeScene } from './scenes/lattice'
import { OrbitScene } from './scenes/orbit'
import { ShardsScene } from './scenes/shards'
import { SoulScene } from './scenes/soul'
import { StreamScene } from './scenes/stream'

const SCENES: Record<SceneName, { component: React.ComponentType; camera: [number, number, number]; fov?: number }> = {
  brain: { component: BrainScene, camera: [0, 0, 6] },
  stream: { component: StreamScene, camera: [0, 0.4, 7], fov: 45 },
  soul: { component: SoulScene, camera: [0, 0, 5.2] },
  lattice: { component: LatticeScene, camera: [0, 0.6, 6] },
  orbit: { component: OrbitScene, camera: [0, 2.4, 6.5] },
  grid: { component: GridScene, camera: [0, 0, 7] },
  shards: { component: ShardsScene, camera: [0, 0, 8.5] },
}

export function Scene({ name }: { name: SceneName }) {
  const { component: Body, camera, fov = 50 } = SCENES[name]
  return (
    <Canvas
      dpr={[1, 1.5]}
      camera={{ position: camera, fov }}
      gl={{ antialias: true, alpha: true, powerPreference: 'low-power' }}
      style={{ background: 'transparent' }}
    >
      <Body />
    </Canvas>
  )
}
