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
  /** How many coins are on the board, for the positions. */
  boardSize?: number
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

/** Rung labels closer than this along the strip are pushed apart so they do not overprint. */
const MIN_RUNG_GAP = 96

/**
 * "$TINY with the market cap of X", as a picture.
 *
 * A versus card: the token on the left with its logo, the yardstick on the
 * right with its own, the multiple between them; under that the result,
 * the price the token would be at that cap; and along the bottom the climb,
 * a log-scale strip from the token's cap to the moon with a rung for each
 * of the coins between, today's yardstick lit. The arithmetic is the whole
 * content, so the fine print says exactly what it is -- today's cap,
 * today's supply -- and the date it was true.
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
    `<linearGradient id="track" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="${MOON.fast}"/><stop offset="1" stop-color="${MOON.accent}"/></linearGradient>`,
    '<clipPath id="disc-fighter"><circle cx="0" cy="0" r="64"/></clipPath>',
    '<clipPath id="disc-token"><circle cx="0" cy="0" r="14"/></clipPath>',
    '</defs>',
    `<rect width="${width}" height="${height}" fill="url(#sky)"/>`,
  )

  // A few stars, fixed by arithmetic rather than random so the same input
  // draws the same picture.
  for (let index = 0; index < 48; index += 1) {
    const x = (index * 293) % width
    const y = (index * 173) % Math.round(height * 0.6)
    const r = index % 5 === 0 ? 1.6 : 1
    parts.push(`<circle cx="${x}" cy="${y}" r="${r}" fill="#ffffff" opacity="${index % 3 === 0 ? 0.35 : 0.18}"/>`)
  }


  // --- the header -----------------------------------------------------------
  parts.push(
    `<text x="${width / 2}" y="60" fill="${MOON.muted}" font-size="20" letter-spacing="4" text-anchor="middle">${escapeXml(`${tokenTag} WITH THE MARKET CAP OF ${targetTag}`)}</text>`,
  )

  // --- the two fighters -----------------------------------------------------
  const ringY = 168
  const leftX = 300
  const rightX = width - 300
  parts.push(fighter(leftX, ringY, token, input.tokenLogo, MOON.fast, position(token, input)))
  parts.push(fighter(rightX, ringY, target.coin, input.targetLogo, MOON.accent, position(target.coin, input)))

  // VS between them, and the multiple under it: the one number the picture
  // is about, in the middle where the eye lands first.
  const midX = width / 2
  parts.push(
    `<circle cx="${midX}" cy="${ringY}" r="36" fill="${MOON.card}" stroke="${MOON.cardBorder}" stroke-width="2"/>`,
    `<text x="${midX}" y="${ringY + 9}" fill="${MOON.text}" font-size="26" font-weight="700" text-anchor="middle" letter-spacing="2">VS</text>`,
    `<text x="${midX}" y="${ringY + 82}" fill="${MOON.up}" font-size="48" font-weight="700" text-anchor="middle">${escapeXml(multiple(target.multiple))}</text>`,
    `<text x="${midX}" y="${ringY + 108}" fill="${MOON.muted}" font-size="16" text-anchor="middle">${escapeXml('the market cap gap')}</text>`,
  )

  // --- the result -----------------------------------------------------------
  const panelX = 120
  const panelW = width - panelX * 2
  const panelY = 366
  const implied = `$${price(target.impliedPriceUsd)}`
  parts.push(
    `<rect x="${panelX}" y="${panelY}" width="${panelW}" height="104" rx="16" fill="${MOON.card}" stroke="${MOON.cardBorder}" stroke-width="1"/>`,
    `<text x="${panelX + 28}" y="${panelY + 32}" fill="${MOON.muted}" font-size="15" letter-spacing="2">${escapeXml(`${tokenTag} AT ${targetTag}'S MARKET CAP`)}</text>`,
    `<text x="${panelX + 28}" y="${panelY + 86}" fill="${MOON.text}" font-size="${implied.length > 11 ? 48 : 56}" font-weight="700">${escapeXml(implied)}</text>`,
    `<text x="${panelX + panelW - 28}" y="${panelY + 32}" fill="${MOON.muted}" font-size="15" letter-spacing="2" text-anchor="end">TODAY</text>`,
    `<text x="${panelX + panelW - 28}" y="${panelY + 86}" fill="${MOON.muted}" font-size="40" font-weight="700" text-anchor="end">${escapeXml(`$${price(token.priceUsd)}`)}</text>`,
  )

  // --- the climb, along the bottom --------------------------------------------
  const trackY = 562
  const trackLeft = 168
  const trackRight = width - 168
  const rungs = input.ladder.filter((entry) => entry.coin.marketCapUsd > token.marketCapUsd)
  const highest = Math.max(target.coin.marketCapUsd, ...rungs.map((entry) => entry.coin.marketCapUsd))
  const lowLog = Math.log10(token.marketCapUsd)
  const highLog = Math.log10(highest)
  const xOf = (capUsd: number): number =>
    highLog === lowLog ? trackLeft : trackLeft + ((Math.log10(capUsd) - lowLog) / (highLog - lowLog)) * (trackRight - trackLeft - 48)

  parts.push(
    `<rect x="${trackLeft}" y="${trackY - 1.5}" width="${trackRight - trackLeft}" height="3" rx="1.5" fill="url(#track)"/>`,
  )
  // The token at the left end, where it is today; the moon at the right.
  parts.push(disc(trackLeft, trackY, 14, token, input.tokenLogo, 'disc-token'))
  parts.push(
    `<text x="${trackLeft - 24}" y="${trackY + 6}" fill="${MOON.text}" font-size="16" font-weight="700" text-anchor="end">${escapeXml(tokenTag)}</text>`,
    `<text x="${trackLeft - 24}" y="${trackY + 26}" fill="${MOON.muted}" font-size="13" text-anchor="end">${escapeXml(compactUsd(token.marketCapUsd))}</text>`,
    `<circle cx="${trackRight + 24}" cy="${trackY}" r="18" fill="${MOON.moon}"/>`,
    `<circle cx="${trackRight + 18}" cy="${trackY - 5}" r="4" fill="${MOON.moonShade}"/>`,
    `<circle cx="${trackRight + 30}" cy="${trackY + 5}" r="5" fill="${MOON.moonShade}"/>`,
  )

  // Rungs, in two lanes so labels can be twice as dense as the strip is
  // wide, and pushed apart within a lane: the strip is log-scaled and two
  // coins a few percent apart would otherwise land on the same pixel.
  const placed = rungs
    .map((entry) => ({ entry, x: xOf(entry.coin.marketCapUsd) }))
    .sort((a, b) => a.x - b.x)
    .map((item, index) => ({ ...item, lane: index % 2 }))
  for (const lane of [0, 1]) {
    let floor = trackLeft + 24
    for (const item of placed.filter((candidate) => candidate.lane === lane)) {
      if (item.x < floor) item.x = floor
      floor = item.x + MIN_RUNG_GAP
    }
  }
  for (const { entry, x, lane } of placed) {
    const lit = entry.coin.id === target.coin.id
    const tag = cashtag(entry.coin.symbol)
    const above = lane === 0
    const tagY = above ? trackY - 34 : trackY + 40
    const detailY = above ? trackY - 16 : trackY + 58
    if (lit) {
      parts.push(`<rect x="${x - 58}" y="${above ? trackY - 52 : trackY + 22}" width="116" height="48" rx="10" fill="${MOON.accentSoft}"/>`)
    }
    parts.push(
      `<line x1="${x}" y1="${trackY}" x2="${x}" y2="${above ? trackY - 10 : trackY + 10}" stroke="${lit ? MOON.accent : MOON.muted}" stroke-width="2"/>`,
      `<circle cx="${x}" cy="${trackY}" r="${lit ? 8 : 5}" fill="${lit ? MOON.accent : MOON.background}" stroke="${lit ? MOON.accent : MOON.muted}" stroke-width="2"/>`,
      `<text x="${x}" y="${tagY}" fill="${lit ? MOON.text : MOON.muted}" font-size="${lit ? 16 : 14}" font-weight="700" text-anchor="middle">${escapeXml(tag)}</text>`,
      `<text x="${x}" y="${detailY}" fill="${lit ? MOON.text : MOON.muted}" font-size="12" text-anchor="middle">${escapeXml(`${compactUsd(entry.coin.marketCapUsd)} · ${multiple(entry.multiple)}`)}</text>`,
    )
  }

  // --- the fine print -------------------------------------------------------
  parts.push(
    `<text x="${panelX}" y="${height - 18}" fill="${MOON.muted}" font-size="13">${escapeXml(`${tokenTag} at its supply today, priced at ${targetTag}'s market cap. Arithmetic, not a forecast. · ${input.boardName}${input.boardSize === undefined ? '' : ` (${input.boardSize} coins)`} · CoinGecko · ${input.date}`)}</text>`,
  )
  if (input.watermark) {
    parts.push(
      `<text x="${width - panelX}" y="${height - 18}" fill="${MOON.muted}" font-size="13" font-weight="700" text-anchor="end">${escapeXml(input.watermark)}</text>`,
    )
  }

  parts.push('</svg>')
  return parts.join('\n')
}

/** One side of the versus: the logo in a coloured ring, the ticker, the name and place, the cap and price. */
function fighter(cx: number, cy: number, coin: ListedCoin, logo: Logo | undefined, ring: string, place: string): string {
  const tag = cashtag(coin.symbol)
  const change =
    coin.change24hPct === undefined
      ? ''
      : ` · <tspan fill="${coin.change24hPct >= 0 ? MOON.up : MOON.down}">${escapeXml(`${coin.change24hPct >= 0 ? '+' : ''}${coin.change24hPct.toFixed(1)}% 24h`)}</tspan>`
  return [
    `<circle cx="${cx}" cy="${cy}" r="70" fill="none" stroke="${ring}" stroke-width="4"/>`,
    disc(cx, cy, 64, coin, logo, 'disc-fighter'),
    `<text x="${cx}" y="${cy + 116}" fill="${MOON.text}" font-size="40" font-weight="700" text-anchor="middle">${escapeXml(tag)}</text>`,
    `<text x="${cx}" y="${cy + 142}" fill="${MOON.muted}" font-size="16" text-anchor="middle">${escapeXml(coin.name + place)}</text>`,
    `<text x="${cx}" y="${cy + 166}" fill="${MOON.text}" font-size="18" text-anchor="middle">${escapeXml(`${compactUsd(coin.marketCapUsd)} mcap`)}${change}</text>`,
  ].join('\n')
}

/** 490.07x reads as 490x; 2.5x keeps its half. */
export function multiple(value: number): string {
  if (!Number.isFinite(value)) return '—'
  if (value >= 100) return `${Math.round(value)}x`
  if (value >= 10) return `${value.toFixed(1).replace(/\.0$/, '')}x`
  return `${value.toFixed(2).replace(/0+$/, '').replace(/\.$/, '')}x`
}

/** "#11 of 360 AI meme coins", or CoinGecko's overall rank when the coin is not from a board. */
function position(coin: ListedCoin, input: MoonChartInput): string {
  if (coin.boardRank !== undefined) return ` · #${coin.boardRank}${input.boardSize === undefined ? '' : ` of ${input.boardSize}`} ${input.boardName}`
  if (coin.rank !== undefined) return ` · #${coin.rank} on CoinGecko`
  return ''
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
