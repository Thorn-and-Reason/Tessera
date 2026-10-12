//! Segment labels choose the more legible ink from the actual opaque fill.
fn luminance(fill: &str) -> f64 {
    [0.2126, 0.7152, 0.0722]
        .into_iter()
        .enumerate()
        .map(|(i, weight)| {
            let channel = u8::from_str_radix(&fill[1 + i * 2..3 + i * 2], 16)
                .expect("fixed palette hex colour") as f64
                / 255.;
            weight
                * if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}
pub(super) fn label(fill: &str) -> &'static str {
    let light = luminance(fill);
    if 1.05 / (light + 0.05) >= (light + 0.05) / 0.05 {
        "white"
    } else {
        "black"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figures::FigurePalette;
    #[test]
    fn every_scale_palette_has_readable_segment_ink() {
        for palette in [
            FigurePalette::Report,
            FigurePalette::Accessible,
            FigurePalette::Monochrome,
        ] {
            for points in [3, 5, 7] {
                for fill in super::super::colours(points, palette) {
                    let light = luminance(fill);
                    let contrast = if label(fill) == "white" {
                        1.05 / (light + 0.05)
                    } else {
                        (light + 0.05) / 0.05
                    };
                    assert!(contrast >= 4.5, "{fill}: contrast {contrast}");
                }
            }
        }
        assert_eq!(label("#737373"), "white");
        assert_eq!(label("#B86732"), "black");
    }
}
