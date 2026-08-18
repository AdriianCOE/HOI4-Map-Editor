pub fn hsl_to_rgb(hsl: [f32; 3]) -> [u8; 3] {
    let [hue, saturation, lightness] = hsl;
    let hue = hue.rem_euclid(360.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let secondary = chroma * (1.0 - ((hue / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match hue {
        hue if hue < 60.0 => (chroma, secondary, 0.0),
        hue if hue < 120.0 => (secondary, chroma, 0.0),
        hue if hue < 180.0 => (0.0, chroma, secondary),
        hue if hue < 240.0 => (0.0, secondary, chroma),
        hue if hue < 300.0 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = lightness - chroma / 2.0;

    [red, green, blue].map(|channel| ((channel + offset).clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::hsl_to_rgb;

    #[test]
    fn converts_primary_and_achromatic_colors() {
        assert_eq!(hsl_to_rgb([0.0, 1.0, 0.5]), [255, 0, 0]);
        assert_eq!(hsl_to_rgb([120.0, 1.0, 0.5]), [0, 255, 0]);
        assert_eq!(hsl_to_rgb([240.0, 1.0, 0.5]), [0, 0, 255]);
        assert_eq!(hsl_to_rgb([30.0, 0.0, 0.5]), [128, 128, 128]);
        assert_eq!(hsl_to_rgb([360.0, 1.0, 0.5]), [255, 0, 0]);
    }
}
