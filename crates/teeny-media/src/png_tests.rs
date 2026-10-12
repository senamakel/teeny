use super::*;

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80" viewBox="0 0 120 80" font-family="'Noto Sans', sans-serif">
<rect x="10" y="10" width="40" height="30" fill="#26a69a"/>
<text x="60" y="60" fill="#d1d4dc" font-size="20">$TINY</text>
</svg>"##;

#[test]
fn rasterises_at_the_svg_size_on_the_chart_background() {
    let png = svg_to_png(SVG).unwrap();
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    let image = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (120, 80));
    // Nothing drawn in the corner: that is the background, not transparency.
    assert_eq!(image.get_pixel(0, 0).0, [0x13, 0x17, 0x22, 0xff]);
    assert_eq!(image.get_pixel(20, 20).0, [0x26, 0xa6, 0x9a, 0xff]);
}

#[test]
fn refuses_text_that_is_not_svg() {
    assert!(svg_to_png("not an svg").is_err());
}
