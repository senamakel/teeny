'use client'

import { useFrame } from '@react-three/fiber'
import { useMemo, useRef } from 'react'
import * as THREE from 'three'
import { ACCENT, useMotion } from '../motion'

const COUNT = 900
const NODES = 9

function build() {
  const t = new Float32Array(COUNT)
  const offset = new Float32Array(COUNT * 3)
  const speed = new Float32Array(COUNT)
  for (let i = 0; i < COUNT; i++) {
    t[i] = Math.random()
    const spread = Math.random() ** 2 * 0.9
    const angle = Math.random() * Math.PI * 2
    offset[i * 3] = 0
    offset[i * 3 + 1] = Math.cos(angle) * spread
    offset[i * 3 + 2] = Math.sin(angle) * spread
    speed[i] = 0.04 + Math.random() * 0.08
  }
  return { t, offset, speed, positions: new Float32Array(COUNT * 3) }
}

const state = build()

/** Thoughts flowing along a line, left to right, with a few that settle into decisions. */
export function StreamScene() {
  const motion = useMotion()
  const points = useRef<THREE.Points>(null)

  const curve = useMemo(
    () =>
      new THREE.CatmullRomCurve3([
        new THREE.Vector3(-7, -0.6, -1),
        new THREE.Vector3(-3.5, 0.9, 0.4),
        new THREE.Vector3(0, -0.4, -0.3),
        new THREE.Vector3(3.5, 0.8, 0.5),
        new THREE.Vector3(7, -0.2, -1),
      ]),
    [],
  )

  const nodes = useMemo(
    () => Array.from({ length: NODES }, (_, i) => curve.getPoint((i + 0.5) / NODES)),
    [curve],
  )

  const v = useMemo(() => new THREE.Vector3(), [])

  useFrame((_, delta) => {
    const dt = delta * motion
    const attr = points.current?.geometry.getAttribute('position') as THREE.BufferAttribute | undefined
    if (!attr) return
    for (let i = 0; i < COUNT; i++) {
      state.t[i] = (state.t[i] + state.speed[i] * dt) % 1
      curve.getPoint(state.t[i], v)
      attr.setXYZ(
        i,
        v.x + state.offset[i * 3],
        v.y + state.offset[i * 3 + 1],
        v.z + state.offset[i * 3 + 2],
      )
    }
    attr.needsUpdate = true
  })

  return (
    <group>
      <points ref={points}>
        <bufferGeometry>
          <bufferAttribute attach="attributes-position" args={[state.positions, 3]} />
        </bufferGeometry>
        <pointsMaterial size={0.06} color={ACCENT} sizeAttenuation transparent opacity={0.75} />
      </points>
      {nodes.map((p, i) => (
        <mesh key={i} position={p}>
          <sphereGeometry args={[0.09, 16, 16]} />
          <meshBasicMaterial color={ACCENT} />
        </mesh>
      ))}
    </group>
  )
}
