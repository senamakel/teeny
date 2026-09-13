'use client'

import { useFrame } from '@react-three/fiber'
import { useMemo, useRef } from 'react'
import * as THREE from 'three'
import { ACCENT, useMotion } from '../motion'

const SIDE = 9
const GAP = 0.5
const COUNT = SIDE * SIDE * SIDE

function build() {
  const positions = new Float32Array(COUNT * 3)
  let i = 0
  const half = ((SIDE - 1) * GAP) / 2
  for (let x = 0; x < SIDE; x++)
    for (let y = 0; y < SIDE; y++)
      for (let z = 0; z < SIDE; z++) {
        positions[i++] = x * GAP - half
        positions[i++] = y * GAP - half
        positions[i++] = z * GAP - half
      }
  const dim = new THREE.Color('#6a6a6a')
  const lit = new THREE.Color(ACCENT)
  const colors = new Float32Array(COUNT * 3)
  for (let j = 0; j < COUNT; j++) {
    colors[j * 3] = dim.r
    colors[j * 3 + 1] = dim.g
    colors[j * 3 + 2] = dim.b
  }
  return { positions, colors, dim, lit }
}

const { positions, colors, dim, lit } = build()
const recalls: { center: THREE.Vector3; life: number }[] = []
let clock = 0

/** A lattice of memories. A recall lights a cluster; it fades as the next one comes. */
export function GridScene() {
  const motion = useMotion()
  const group = useRef<THREE.Group>(null)
  const points = useRef<THREE.Points>(null)


  const v = useMemo(() => new THREE.Vector3(), [])
  const c = useMemo(() => new THREE.Color(), [])

  useFrame((_, delta) => {
    const dt = delta * motion
    clock += dt
    if (group.current) {
      group.current.rotation.y += dt * 0.06
      group.current.rotation.x = 0.35 + Math.sin(clock * 0.2) * 0.1
    }
    const attr = points.current?.geometry.getAttribute('color') as THREE.BufferAttribute | undefined
    if (!attr) return
    if (motion > 0 && Math.random() < dt * 2.2) {
      const half = ((SIDE - 1) * GAP) / 2
      recalls.push({
        center: new THREE.Vector3(
          (Math.random() * 2 - 1) * half,
          (Math.random() * 2 - 1) * half,
          (Math.random() * 2 - 1) * half,
        ),
        life: 1,
      })
    }
    for (let k = recalls.length - 1; k >= 0; k--) if (recalls[k].life <= 0) recalls.splice(k, 1)
    for (let j = 0; j < COUNT; j++) {
      v.set(positions[j * 3], positions[j * 3 + 1], positions[j * 3 + 2])
      let glow = 0
      for (const r of recalls) {
        const d = v.distanceTo(r.center)
        glow = Math.max(glow, Math.max(0, 1 - d / 1.5) * r.life)
      }
      c.copy(dim).lerp(lit, glow)
      attr.setXYZ(j, c.r, c.g, c.b)
    }
    for (const r of recalls) r.life -= dt * 0.8
    attr.needsUpdate = true
  })

  return (
    <group ref={group}>
      <points ref={points}>
        <bufferGeometry>
          <bufferAttribute attach="attributes-position" args={[positions, 3]} />
          <bufferAttribute attach="attributes-color" args={[colors, 3]} />
        </bufferGeometry>
        <pointsMaterial size={0.075} vertexColors sizeAttenuation />
      </points>
    </group>
  )
}
