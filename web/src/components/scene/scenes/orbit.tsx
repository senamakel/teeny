'use client'

import { useFrame } from '@react-three/fiber'
import { useMemo, useRef } from 'react'
import * as THREE from 'three'
import { ACCENT, ACCENT_DIM, INK, useMotion } from '../motion'

/** Five rings, one per tool group, each carrying its tools around a core. */
const RINGS = [
  { count: 2, radius: 1.1, tilt: 0.2, speed: 0.5 },
  { count: 4, radius: 1.6, tilt: -0.5, speed: 0.38 },
  { count: 12, radius: 2.3, tilt: 0.9, speed: 0.22 },
  { count: 5, radius: 2.9, tilt: -1.1, speed: 0.17 },
  { count: 2, radius: 3.4, tilt: 0.4, speed: 0.13 },
]

function Ring({ count, radius, tilt, speed, motion }: (typeof RINGS)[number] & { motion: number }) {
  const group = useRef<THREE.Group>(null)
  const items = useMemo(
    () => Array.from({ length: count }, (_, i) => (i / count) * Math.PI * 2),
    [count],
  )
  useFrame((_, delta) => {
    if (group.current) group.current.rotation.y += delta * speed * motion
  })
  return (
    <group rotation={[tilt, 0, tilt * 0.4]}>
      <mesh rotation={[Math.PI / 2, 0, 0]}>
        <torusGeometry args={[radius, 0.005, 6, 160]} />
        <meshBasicMaterial color={INK} transparent opacity={0.5} />
      </mesh>
      <group ref={group}>
        {items.map((angle, i) => (
          <mesh key={i} position={[Math.cos(angle) * radius, 0, Math.sin(angle) * radius]}>
            <boxGeometry args={[0.11, 0.11, 0.11]} />
            <meshStandardMaterial color={i % 3 === 0 ? ACCENT : ACCENT_DIM} roughness={0.5} />
          </mesh>
        ))}
      </group>
    </group>
  )
}

export function OrbitScene() {
  const motion = useMotion()
  return (
    <>
      <ambientLight intensity={0.7} />
      <pointLight position={[4, 4, 4]} intensity={35} color="#ffffff" />
      <mesh>
        <sphereGeometry args={[0.35, 32, 32]} />
        <meshStandardMaterial color={ACCENT} emissive={ACCENT} emissiveIntensity={0.6} />
      </mesh>
      {RINGS.map((ring, i) => (
        <Ring key={i} {...ring} motion={motion} />
      ))}
    </>
  )
}
