'use client'

import { useFrame } from '@react-three/fiber'
import { useRef } from 'react'
import * as THREE from 'three'
import { ACCENT, ACCENT_DIM, INK, useMotion } from '../motion'

/** A small solid held inside a cage of rings. The rings turn; the core does not move. */
export function LatticeScene() {
  const motion = useMotion()
  const cage = useRef<THREE.Group>(null)
  const ringA = useRef<THREE.Mesh>(null)
  const ringB = useRef<THREE.Mesh>(null)
  const ringC = useRef<THREE.Mesh>(null)

  useFrame((_, delta) => {
    const dt = delta * motion
    if (cage.current) cage.current.rotation.y += dt * 0.12
    if (ringA.current) ringA.current.rotation.x += dt * 0.35
    if (ringB.current) ringB.current.rotation.y += dt * 0.28
    if (ringC.current) ringC.current.rotation.z += dt * 0.22
  })

  return (
    <>
      <ambientLight intensity={0.5} />
      <pointLight position={[3, 4, 3]} intensity={30} color={ACCENT} />
      <group ref={cage}>
        <mesh>
          <icosahedronGeometry args={[2.2, 1]} />
          <meshBasicMaterial color={INK} wireframe transparent opacity={0.35} />
        </mesh>
        <mesh ref={ringA}>
          <torusGeometry args={[1.7, 0.012, 8, 128]} />
          <meshBasicMaterial color={ACCENT} />
        </mesh>
        <mesh ref={ringB} rotation={[Math.PI / 2, 0, 0]}>
          <torusGeometry args={[1.45, 0.012, 8, 128]} />
          <meshBasicMaterial color={ACCENT_DIM} />
        </mesh>
        <mesh ref={ringC} rotation={[0, Math.PI / 2, 0]}>
          <torusGeometry args={[1.2, 0.012, 8, 128]} />
          <meshBasicMaterial color={ACCENT} />
        </mesh>
      </group>
      <mesh>
        <octahedronGeometry args={[0.45, 0]} />
        <meshStandardMaterial color={ACCENT} emissive={ACCENT} emissiveIntensity={0.4} roughness={0.4} />
      </mesh>
    </>
  )
}
