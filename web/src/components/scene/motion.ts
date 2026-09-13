'use client'

import { useEffect, useState } from 'react'

/** 1 normally, 0 when the viewer asked for reduced motion. Scenes multiply their speed by it. */
export function useMotion(): number {
  const [motion, setMotion] = useState(1)
  useEffect(() => {
    const query = window.matchMedia('(prefers-reduced-motion: reduce)')
    const apply = () => setMotion(query.matches ? 0 : 1)
    apply()
    query.addEventListener('change', apply)
    return () => query.removeEventListener('change', apply)
  }, [])
  return motion
}

export const ACCENT = '#f2a544'
export const ACCENT_DIM = '#8a5a1e'
export const INK = '#6b6b6b'
