/**
 * Number formatting for a chart nobody can zoom into.
 *
 * A memecoin price is 0.0007472834089 and a market cap is 730709.30528831.
 * Printed raw, the first is unreadable and the second is a lie about precision.
 */

/** $730.7K, $1.24M, $12.3B. */
export function compactUsd(value: number): string {
  const sign = value < 0 ? '-' : ''
  const size = Math.abs(value)
  const [divisor, suffix] =
    size >= 1e9 ? [1e9, 'B'] : size >= 1e6 ? [1e6, 'M'] : size >= 1e3 ? [1e3, 'K'] : [1, '']
  const scaled = size / divisor
  // A market cap of 730,709 reads as $730.7K, not $731K: on a chart posted at
  // this size the tenth of a percent is the part people compare day to day.
  // Plain dollars keep no decimals at all, so $999 does not print as $999.0.
  const places = scaled >= 100 ? (suffix ? 1 : 0) : scaled >= 10 ? 1 : 2
  return `${sign}$${scaled.toFixed(places)}${suffix}`
}

/**
 * A price, with enough significant figures to be useful and no more.
 *
 * Sub-cent prices get four significant digits, so 0.0007472834089 prints as
 * 0.0007473 rather than as 0.00 or as the whole float.
 */
export function price(value: number): string {
  if (!Number.isFinite(value)) return '—'
  const size = Math.abs(value)
  if (size === 0) return '0'
  if (size >= 1_000) return value.toLocaleString('en-US', { maximumFractionDigits: 2 })
  if (size >= 1) return value.toFixed(4).replace(/0+$/, '').replace(/\.$/, '')
  const decimals = Math.min(Math.max(Math.ceil(-Math.log10(size)) + 3, 4), 12)
  return value.toFixed(decimals).replace(/0+$/, '').replace(/\.$/, '')
}

export function signedPercent(value: number): string {
  return `${value >= 0 ? '+' : ''}${value.toFixed(2)}%`
}

/** Axis labels. Intraday charts want the clock; daily charts want the date. */
export function axisTime(unixSeconds: number, timeframe: string): string {
  const date = new Date(unixSeconds * 1_000)
  const day = `${date.getUTCDate()}`.padStart(2, '0')
  const month = date.toLocaleString('en-US', { month: 'short', timeZone: 'UTC' })
  if (timeframe === '1d') return `${day} ${month}`
  const hours = `${date.getUTCHours()}`.padStart(2, '0')
  const minutes = `${date.getUTCMinutes()}`.padStart(2, '0')
  return `${day} ${month} ${hours}:${minutes}`
}

/** SVG has five characters that cannot appear in text; a token name may contain all of them. */
export function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;')
}
