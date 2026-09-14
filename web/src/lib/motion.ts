'use client'

import { useSyncExternalStore } from "react"

const QUERY = "(prefers-reduced-motion: reduce)"

function subscribe(onChange: () => void): () => void {
  const media = window.matchMedia?.(QUERY)
  media?.addEventListener("change", onChange)
  return () => media?.removeEventListener("change", onChange)
}

/** Whether the viewer asked for less motion. False on the server, so the first paint is still. */
export function useReducedMotion(): boolean {
  return useSyncExternalStore(subscribe, () => window.matchMedia?.(QUERY).matches ?? false, () => false)
}
