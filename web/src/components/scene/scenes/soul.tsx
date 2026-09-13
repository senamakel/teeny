'use client'

import { Float, MeshDistortMaterial } from '@react-three/drei'
import { useFrame } from '@react-three/fiber'
import { useRef } from 'react'
import * as THREE from 'three'
import { ACCENT, useMotion } from '../motion'

/** One warm, soft, slightly restless blob. Smol. */
export function SoulScene() {
  const motion = useMotion()
  const mesh = useRef<THREE.Mesh>(null)

  useFrame((_, delta) => {
    if (mesh.current) mesh.current.rotation.y += delta * 0.15 * motion
  })

  return (
    <>
      <ambientLight intensity={0.9} />
      <pointLight position={[3, 3, 4]} intensity={60} color="#ffe2b8" />
      <pointLight position={[-4, -2, 2]} intensity={12} color="#ffffff" />
      <Float speed={motion ? 1.4 : 0} rotationIntensity={0.3} floatIntensity={0.8}>
        <mesh ref={mesh}>
          <icosahedronGeometry args={[1.6, 64]} />
          <MeshDistortMaterial
            color={ACCENT}
            emissive={ACCENT}
            emissiveIntensity={0.35}
            roughness={0.35}
            metalness={0.1}
            distort={motion ? 0.32 : 0.15}
            speed={motion ? 1.6 : 0}
          />
        </mesh>
      </Float>
    </>
  )
}
