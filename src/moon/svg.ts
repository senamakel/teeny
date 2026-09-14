import { compactUsd, escapeXml, price } from '../chart/format.js'
import { THEME } from '../chart/theme.js'
import type { Rung } from './ladder.js'
import type { ListedCoin } from './leaderboard.js'

export interface Logo {
  data: Uint8Array
  mimeType: string
}

export interface MoonChartInput {
  token: ListedCoin
  /** Today's target, the headline of the picture. */
  target: Rung
  /** The rest of the climb, ascending; the target is among them. */
  ladder: Rung[]
  /** Which leaderboard the targets came from, for the fine print. */
  boardName: string
  /** ISO date, for the fine print. */
  date: string
  tokenLogo?: Logo
  targetLogo?: Logo
  watermark?: string
  width?: number
  height?: number
}

/** The moon's palette, on top of the chart's, so the two posts look like one account. */
export const MOON = {
  ...THEME,
  accent: '#7c6cff',
  accentSoft: '#7c6cff33',
  moon: '#f5e6a8',
  moonShade: '#d9c98a',
  card: '#1b2030',
  cardBorder: '#2a2e39',
} as const

/** Rungs closer than this on the track are pushed apart so their labels do not overprint. */
const MIN_RUNG_GAP = 46

/**
 * "$TINY with the market cap of X", as a picture.
 *
 * Left, the headline: the target with its logo, the price the token would
 * be at that cap, and the multiple. Right, the climb: a track from the
 * token's own cap at the bottom to the moon at the top, on a log scale,
 * with a rung for each of the coins between, and today's target lit. The
 * arithmetic is the whole content, so the fine print says exactly what it
 * is -- today's cap, today's supply -- and the date it was true.
 *
 * SVG text for the same reason the candlestick chart is: the tests assert
 * on it and a diff shows what moved. Logos are embedded as data URIs,
 * which the rasteriser reads; without one the coin gets a disc with its
 * first letter, which is what every exchange does when a logo is missing.
 */
export function renderMoonSvg(input: MoonChartInput): string {
  const width = input.width ?? 1_200
  const height = input.height ?? 675
  const { token, target } = input
  const tokenTag = cashtag(token.symbol)
  const targetTag = cashtag(target.coin.symbol)
  const parts: string[] = []

  parts.push(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="${MOON.font}">`,
    '<defs>',
    `<linearGradient id="sky" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="${MOON.background}"/><stop offset="1" stop-color="#191d33"/></linearGradient>`,
    `<linearGradient id="track" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="${MOON.fast}"/><stop offset="1" stop-color="${MOON.accent}"/></linearGradient>`,
    '<clipPath id="disc-target"><circle cx="0" cy="0" r="30"/></clipPath>',
    '<clipPath id="disc-token"><circle cx="0" cy="0" r="18"/></clipPath>',
    '</defs>',
    `<rect width="${width}" height="${height}" fill="url(#sky)"/>`,
  )

  // A few stars, fixed by arithmetic rather than random so the same input
  // draws the same picture.
  for (let index = 0; index < 40; index += 1) {
    const x = (index * 293) % width
    const y = (index * 173) % Math.round(height * 0.7)
    const r = index % 5 === 0 ? 1.6 : 1
    parts.push(`<circle cx="${x}" cy="${y}" r="${r}" fill="#ffffff" opacity="${index % 3 === 0 ? 0.35 : 0.18}"/>`)
  }

  if (input.watermark) {
    parts.push(
      `<text x="${width / 2}" y="${height / 2 + 24}" fill="${MOON.watermark}" font-size="88" font-weight="700" text-anchor="middle">${escapeXml(input.watermark)}</text>`,
    )
  }

  // --- the headline, left ---------------------------------------------------
  const left = 64
  parts.push(
    `<text x="${left}" y="88" fill="${MOON.muted}" font-size="20" letter-spacing="3">${escapeXml(`${tokenTag} WITH THE MARKET CAP OF`)}</text>`,
  )
  parts.push(disc(left + 30, 150, 30, target.coin, input.targetLogo, 'disc-target'))
  parts.push(
    `<text x="${left + 78}" y="162" fill="${MOON.text}" font-size="52" font-weight="700">${escapeXml(targetTag)}</text>`,
    `<text x="${left + 78}" y="192" fill="${MOON.muted}" font-size="20">${escapeXml(target.coin.name)}${target.coin.rank === undefined ? '' : escapeXml(` · #${target.coin.rank} on CoinGecko`)}</text>`,
  )

  const implied = `$${price(target.impliedPriceUsd)}`
  parts.push(
    `<text x="${left}" y="322" fill="${MOON.text}" font-size="${implied.length > 11 ? 72 : 88}" font-weight="700">${escapeXml(implied)}</text>`,
    `<text x="${left}" y="374" fill="${MOON.up}" font-size="40" font-weight="700">${escapeXml(multiple(target.multiple))}</text>`,
    `<text x="${left + 4 + multiple(target.multiple).length * 22}" y="374" fill="${MOON.muted}" font-size="22">${escapeXml(`from $${price(token.priceUsd)} today`)}</text>`,
  )

  // Two cards: where we are, where they are.
  const cardY = 424
  parts.push(
    card(left, cardY, 232, `${tokenTag} MCAP`, compactUsd(token.marketCapUsd), token.change24hPct),
    card(left + 252, cardY, 232, `${targetTag} MCAP`, compactUsd(target.coin.marketCapUsd), target.coin.change24hPct),
  )

  parts.push(
    `<text x="${left}" y="${height - 66}" fill="${MOON.muted}" font-size="16">${escapeXml(`${tokenTag} at its supply today, priced at ${targetTag}'s market cap. Arithmetic, not a forecast.`)}</text>`,
    `<text x="${left}" y="${height - 42}" fill="${MOON.muted}" font-size="16">${escapeXml(`${input.boardName} · CoinGecko · ${input.date}`)}</text>`,
  )

  // --- the climb, right -----------------------------------------------------
  const trackX = 700
  const top = 96
  const bottom = height - 84
  const rungs = input.ladder.filter((entry) => entry.coin.marketCapUsd > token.marketCapUsd)
  const highest = Math.max(target.coin.marketCapUsd, ...rungs.map((entry) => entry.coin.marketCapUsd))
  const lowLog = Math.log10(token.marketCapUsd)
  const highLog = Math.log10(highest)
  const yOf = (capUsd: number): number =>
    highLog === lowLog ? bottom : bottom - ((Math.log10(capUsd) - lowLog) / (highLog - lowLog)) * (bottom - top - 40)

  parts.push(
    // A rect rather than a line: a gradient on a vertical line has a
    // zero-width bounding box and paints nothing.
    `<rect x="${trackX - 1.5}" y="${top + 30}" width="3" height="${bottom - top - 30}" rx="1.5" fill="url(#track)"/>`,
  )
  // The moon.
  parts.push(
    `<circle cx="${trackX}" cy="${top}" r="26" fill="${MOON.moon}"/>`,
    `<circle cx="${trackX - 8}" cy="${top - 6}" r="5" fill="${MOON.moonShade}"/>`,
    `<circle cx="${trackX + 9}" cy="${top + 7}" r="7" fill="${MOON.moonShade}"/>`,
    `<circle cx="${trackX - 3}" cy="${top + 12}" r="3" fill="${MOON.moonShade}"/>`,
  )
  // The token, at the bottom, where it is today.
  parts.push(disc(trackX, bottom, 18, token, input.tokenLogo, 'disc-token'))
  parts.push(
    `<text x="${trackX + 36}" y="${bottom + 6}" fill="${MOON.text}" font-size="20" font-weight="700">${escapeXml(tokenTag)}</text>`,
    `<text x="${trackX + 36 + tokenTag.length * 13}" y="${bottom + 6}" fill="${MOON.muted}" font-size="18">${escapeXml(` now · ${compactUsd(token.marketCapUsd)} · $${price(token.priceUsd)}`)}</text>`,
  )

  // Rungs, pushed apart from the bottom so no two labels overprint; the
  // track is log-scaled and two coins a few percent apart would otherwise
  // land on the same pixel.
  const placed = rungs
    .map((entry) => ({ entry, y: yOf(entry.coin.marketCapUsd) }))
    .sort((a, b) => b.y - a.y)
  let floor = bottom - MIN_RUNG_GAP
  for (const item of placed) {
    if (item.y > floor) item.y = floor
    floor = item.y - MIN_RUNG_GAP
  }
  for (const { entry, y } of placed) {
    const lit = entry.coin.id === target.coin.id
    const tag = cashtag(entry.coin.symbol)
    if (lit) {
      parts.push(
        `<rect x="${trackX - 16}" y="${y - 22}" width="${width - trackX - 48}" height="44" rx="10" fill="${MOON.accentSoft}"/>`,
      )
    }
    parts.push(
      `<circle cx="${trackX}" cy="${y}" r="${lit ? 9 : 6}" fill="${lit ? MOON.accent : MOON.background}" stroke="${lit ? MOON.accent : MOON.muted}" stroke-width="2"/>`,
      `<text x="${trackX + 28}" y="${y + 6}" fill="${lit ? MOON.text : MOON.muted}" font-size="${lit ? 20 : 17}" font-weight="700">${escapeXml(tag)}</text>`,
      `<text x="${width - 48}" y="${y + 6}" fill="${lit ? MOON.text : MOON.muted}" font-size="${lit ? 18 : 16}" text-anchor="end">${escapeXml(`${compactUsd(entry.coin.marketCapUsd)}  →  $${price(entry.impliedPriceUsd)}  ·  ${multiple(entry.multiple)}`)}</text>`,
    )
  }

  parts.push('</svg>')
  return parts.join('\n')
}

/** 490.07x reads as 490x; 2.5x keeps its half. */
export function multiple(value: number): string {
  if (!Number.isFinite(value)) return '—'
  if (value >= 100) return `${Math.round(value)}x`
  if (value >= 10) return `${value.toFixed(1).replace(/\.0$/, '')}x`
  return `${value.toFixed(2).replace(/0+$/, '').replace(/\.$/, '')}x`
}

function cashtag(symbol: string): string {
  return symbol.startsWith('$') ? symbol : `$${symbol}`
}

/** A coin's logo in a circle, or a disc with its initial when there is none. */
function disc(cx: number, cy: number, r: number, coin: ListedCoin, logo: Logo | undefined, clip: string): string {
  if (logo) {
    const href = `data:${logo.mimeType};base64,${Buffer.from(logo.data).toString('base64')}`
    return (
      `<g transform="translate(${cx} ${cy})" clip-path="url(#${clip})">` +
      `<circle cx="0" cy="0" r="${r}" fill="${MOON.card}"/>` +
      `<image href="${href}" x="${-r}" y="${-r}" width="${r * 2}" height="${r * 2}" preserveAspectRatio="xMidYMid slice"/>` +
      '</g>'
    )
  }
  const letter = coin.symbol.replace(/^\$/, '').charAt(0).toUpperCase() || '?'
  return (
    `<circle cx="${cx}" cy="${cy}" r="${r}" fill="${MOON.card}" stroke="${MOON.cardBorder}" stroke-width="2"/>` +
    `<text x="${cx}" y="${cy + r * 0.36}" fill="${MOON.text}" font-size="${r}" font-weight="700" text-anchor="middle">${escapeXml(letter)}</text>`
  )
}

function card(x: number, y: number, w: number, label: string, value: string, change: number | undefined): string {
  const delta =
    change === undefined
      ? ''
      : `<text x="${x + w - 16}" y="${y + 30}" fill="${change >= 0 ? MOON.up : MOON.down}" font-size="14" text-anchor="end">${escapeXml(`${change >= 0 ? '+' : ''}${change.toFixed(1)}% 24h`)}</text>`
  return (
    `<rect x="${x}" y="${y}" width="${w}" height="92" rx="12" fill="${MOON.card}" stroke="${MOON.cardBorder}" stroke-width="1"/>` +
    `<text x="${x + 16}" y="${y + 30}" fill="${MOON.muted}" font-size="13" letter-spacing="1.5">${escapeXml(label)}</text>` +
    `<text x="${x + 16}" y="${y + 72}" fill="${MOON.text}" font-size="30" font-weight="700">${escapeXml(value)}</text>` +
    delta
  )
}
