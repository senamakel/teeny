import { Resvg } from '@resvg/resvg-js'

/**
 * SVG to PNG.
 *
 * X will not accept an SVG as media, so the chart has to be rasterised
 * somewhere. resvg is the whole reason there is a native dependency in this
 * repo; it ships prebuilt binaries for musl, which is what the runtime image
 * is, and it renders text through the system fonts -- so the image has fonts
 * installed alongside it. Without them every label silently disappears and the
 * chart posts as candles with no axis, which is why the runner installs
 * font-noto and this asks for it by name.
 */
export function svgToPng(svg: string, options: { width?: number } = {}): Buffer {
  const renderer = new Resvg(svg, {
    ...(options.width ? { fitTo: { mode: 'width' as const, value: options.width } } : {}),
    font: {
      loadSystemFonts: true,
      defaultFontFamily: 'Noto Sans',
    },
    background: '#131722',
  })
  return renderer.render().asPng()
}
