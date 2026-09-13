'use client'

import { useFrame } from '@react-three/fiber'
import { useRef } from 'react'
import * as THREE from 'three'
import { ACCENT, ACCENT_DIM, useMotion } from '../motion'

const COUNT = 520
const RADIUS = 2.1
const LINK_DISTANCE = 0.55

function build() {
  const positions = new Float32Array(COUNT * 3)
  const golden = Math.PI * (3 - Math.sqrt(5))
  for (let i = 0; i < COUNT; i++) {
    const y = 1 - (i / (COUNT - 1)) * 2
    const r = Math.sqrt(1 - y * y)
    const theta = golden * i
    // Slight jitter so it reads as tissue rather than a lattice.
    const jitter = 0.92 + Math.random() * 0.16
    positions[i * 3] = Math.cos(theta) * r * RADIUS * jitter
    positions[i * 3 + 1] = y * RADIUS * jitter
    positions[i * 3 + 2] = Math.sin(theta) * r * RADIUS * jitter
  }
  const pairs: number[] = []
  for (let i = 0; i < COUNT; i++) {
    for (let j = i + 1; j < COUNT; j++) {
      const dx = positions[i * 3] - positions[j * 3]
      const dy = positions[i * 3 + 1] - positions[j * 3 + 1]
      const dz = positions[i * 3 + 2] - positions[j * 3 + 2]
      if (dx * dx + dy * dy + dz * dz < LINK_DISTANCE * LINK_DISTANCE) {
        pairs.push(
          positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2],
          positions[j * 3], positions[j * 3 + 1], positions[j * 3 + 2],
        )
      }
    }
  }
  const base = new THREE.Color(ACCENT_DIM)
  const colors = new Float32Array(COUNT * 3)
  for (let i = 0; i < COUNT; i++) {
    colors[i * 3] = base.r
    colors[i * 3 + 1] = base.g
    colors[i * 3 + 2] = base.b
  }
  return { positions, links: new Float32Array(pairs), colors, base }
}

const DATA = build()
const BRIGHT = new THREE.Color(ACCENT)
/** Neurons currently lit, and how much life each has left. */
const firing: { index: number; life: number }[] = []

/** Points on a sphere, joined to their near neighbours. A brain, loosely. */
export function BrainScene() {
  const motion = useMotion()
  const group = useRef<THREE.Group>(null)
  const points = useRef<THREE.Points>(null)


  useFrame((_, delta) => {
    const dt = delta * motion
    if (group.current) {
      group.current.rotation.y += dt * 0.08
      group.current.rotation.x = Math.sin(performance.now() * 0.0001) * 0.15
    }
    const geometry = points.current?.geometry
    const attr = geometry?.getAttribute('color') as THREE.BufferAttribute | undefined
    if (!attr) return
    // A few neurons fire at a time and fade back down.
    if (motion > 0 && Math.random() < 0.35) {
      firing.push({ index: Math.floor(Math.random() * COUNT), life: 1 })
    }
    for (let i = firing.length - 1; i >= 0; i--) if (firing[i].life <= 0) firing.splice(i, 1)
    for (let i = 0; i < COUNT; i++) attr.setXYZ(i, DATA.base.r, DATA.base.g, DATA.base.b)
    for (const f of firing) {
      const c = DATA.base.clone().lerp(BRIGHT, f.life)
      attr.setXYZ(f.index, c.r, c.g, c.b)
      f.life -= dt * 1.4
    }
    attr.needsUpdate = true
  })

  return (
    <group ref={group}>
      <points ref={points}>
        <bufferGeometry>
          <bufferAttribute attach="attributes-position" args={[DATA.positions, 3]} />
          <bufferAttribute attach="attributes-color" args={[DATA.colors, 3]} />
        </bufferGeometry>
        <pointsMaterial size={0.05} vertexColors sizeAttenuation transparent opacity={0.95} />
      </points>
      <lineSegments>
        <bufferGeometry>
          <bufferAttribute attach="attributes-position" args={[DATA.links, 3]} />
        </bufferGeometry>
        <lineBasicMaterial color={ACCENT_DIM} transparent opacity={0.28} />
      </lineSegments>
    </group>
  )
}
