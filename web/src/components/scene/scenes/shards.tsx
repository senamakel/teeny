'use client'

import { useFrame } from '@react-three/fiber'
import { useMemo, useRef } from 'react'
import * as THREE from 'three'
import { ACCENT, ACCENT_DIM, useMotion } from '../motion'

const COUNT = 48

function build() {
  const golden = Math.PI * (3 - Math.sqrt(5))
  return Array.from({ length: COUNT }, (_, i) => {
    const y = 1 - (i / (COUNT - 1)) * 2
    const r = Math.sqrt(1 - y * y)
    const theta = golden * i
    const dir = new THREE.Vector3(Math.cos(theta) * r, y, Math.sin(theta) * r)
    return {
      dir,
      home: dir.clone().multiplyScalar(1.5),
      away: dir.clone().multiplyScalar(2.4 + Math.random() * 1.2),
      spin: new THREE.Euler(Math.random() * 3, Math.random() * 3, Math.random() * 3),
      phase: Math.random() * 0.4,
      accent: i % 4 === 0,
    }
  })
}

const shards = build()
let clock = 0

/** A sphere that keeps coming apart and putting itself back together. */
export function ShardsScene() {
  const motion = useMotion()
  const group = useRef<THREE.Group>(null)
  const meshes = useRef<(THREE.Mesh | null)[]>([])


  const tmp = useMemo(() => new THREE.Vector3(), [])

  useFrame((_, delta) => {
    const dt = delta * motion
    clock += dt
    if (group.current) group.current.rotation.y += dt * 0.1
    // 0 = whole, 1 = broken, easing both ways. Breaks fast, mends slowly.
    const cycle = (clock * 0.18) % 1
    const broken = cycle < 0.25 ? cycle / 0.25 : 1 - (cycle - 0.25) / 0.75
    const eased = broken * broken * (3 - 2 * broken)
    shards.forEach((s, i) => {
      const m = meshes.current[i]
      if (!m) return
      const k = Math.min(1, Math.max(0, eased + s.phase * (eased - 0.5)))
      tmp.copy(s.home).lerp(s.away, k)
      m.position.copy(tmp)
      m.rotation.set(s.spin.x + k * 1.2, s.spin.y + k * 0.8, s.spin.z)
    })
  })

  return (
    <>
      <ambientLight intensity={0.5} />
      <pointLight position={[3, 4, 3]} intensity={30} color="#ffffff" />
      <pointLight position={[-3, -2, 2]} intensity={16} color={ACCENT} />
      <group ref={group}>
        {shards.map((s, i) => (
          <mesh key={i} ref={(el) => { meshes.current[i] = el }} position={s.home}>
            <tetrahedronGeometry args={[0.32, 0]} />
            <meshStandardMaterial
              color={s.accent ? ACCENT : '#2a2a2a'}
              emissive={s.accent ? ACCENT_DIM : '#000000'}
              roughness={0.55}
              flatShading
            />
          </mesh>
        ))}
      </group>
    </>
  )
}
