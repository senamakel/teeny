/**
 * The palette, kept in one place so a chart posted today looks like the one
 * posted last week. It is TradingView's dark theme on purpose: it is what the
 * timeline expects a chart to look like, and a chart that looks unfamiliar
 * reads as fake before anybody has looked at the candles.
 */
export const THEME = {
  background: '#131722',
  panel: '#131722',
  grid: '#1e222d',
  axis: '#2a2e39',
  text: '#d1d4dc',
  muted: '#787b86',
  up: '#26a69a',
  down: '#ef5350',
  upVolume: '#26a69a55',
  downVolume: '#ef535055',
  fast: '#f0b90b',
  slow: '#2962ff',
  level: '#787b8688',
  watermark: '#ffffff10',
  font: "'Noto Sans', 'DejaVu Sans', 'Helvetica Neue', Helvetica, Arial, sans-serif",
} as const
