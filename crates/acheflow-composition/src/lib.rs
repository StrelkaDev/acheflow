mod filters;
pub mod gpu;
#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
pub mod headless;
// Reference CPU processing for fixed-effect regression tests.
use acheflow_media_api::workspace::{EffectKind, LayerEffect, Resolution, SourceCrop};
use image::{GenericImageView, RgbImage, Rgba, RgbaImage, imageops};

/// Crop in decode-canvas pixels; scale the crop for lower resolution previews.
/// API boundaries validate crop and effect ranges before calling this function.
pub fn prepare_source(
    source: &RgbImage,
    size: &Resolution,
    crop: SourceCrop,
    effects: &[LayerEffect],
) -> RgbaImage {
    let w = source.width();
    let h = source.height();
    let x = (u64::from(crop.left) * u64::from(w) / u64::from(size.width.max(1)))
        .min(u64::from(w.saturating_sub(1))) as u32;
    let y = (u64::from(crop.top) * u64::from(h) / u64::from(size.height.max(1)))
        .min(u64::from(h.saturating_sub(1))) as u32;
    let right = (u64::from(crop.right) * u64::from(w) / u64::from(size.width.max(1))) as u32;
    let bottom = (u64::from(crop.bottom) * u64::from(h) / u64::from(size.height.max(1))) as u32;
    let cropped = imageops::crop_imm(
        source,
        x,
        y,
        w.saturating_sub(x).saturating_sub(right).max(1),
        h.saturating_sub(y).saturating_sub(bottom).max(1),
    );
    let mut image = RgbaImage::from_fn(cropped.width(), cropped.height(), |x, y| {
        let [r, g, b] = cropped.get_pixel(x, y).0;
        Rgba([r, g, b, 255])
    });
    for effect in effects.iter().filter(|e| e.enabled) {
        if effect.kind == EffectKind::Blur {
            let radius = effect.amount as f32 * w as f32 / size.width.max(1) as f32;
            if radius > 0. {
                image = imageops::blur(&image, radius);
            }
            continue;
        }
        let amount = effect.amount as f32 / 100.;
        for p in image.pixels_mut() {
            let rgb = [p[0] as f32, p[1] as f32, p[2] as f32];
            let luma = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
            for (channel, value) in rgb.into_iter().enumerate() {
                let value = match effect.kind {
                    EffectKind::Brightness => value * amount,
                    EffectKind::Contrast => (value - 127.5) * amount + 127.5,
                    EffectKind::Saturation => luma + (value - luma) * amount,
                    EffectKind::Grayscale => value + (luma - value) * amount,
                    _ => value,
                };
                p[channel] = value.round().clamp(0., 255.) as u8;
            }
            if effect.kind == EffectKind::Opacity {
                p[3] = (p[3] as f32 * amount).round().clamp(0., 255.) as u8;
            }
        }
    }
    image
}

/// Source-over onto the opaque output canvas, preserving lower layers.
pub fn composite(canvas: &mut RgbImage, layer: &RgbaImage, x: u32, y: u32) {
    for sy in 0..layer.height().min(canvas.height().saturating_sub(y)) {
        for sx in 0..layer.width().min(canvas.width().saturating_sub(x)) {
            let source = layer.get_pixel(sx, sy);
            let alpha = u32::from(source[3]);
            if alpha == 0 {
                continue;
            }
            let target = canvas.get_pixel_mut(x + sx, y + sy);
            if alpha == 255 {
                target.0 = [source[0], source[1], source[2]];
                continue;
            }
            for c in 0..3 {
                target[c] =
                    ((u32::from(source[c]) * alpha + u32::from(target[c]) * (255 - alpha) + 127)
                        / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;
    #[test]
    fn crop_selects_source_pixels_and_scaled_preview_uses_same_region() {
        let source = RgbImage::from_fn(8, 4, |x, _| {
            if x < 4 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        });
        let size = Resolution {
            width: 8,
            height: 4,
        };
        let crop = SourceCrop {
            left: 4,
            ..Default::default()
        };
        let full = prepare_source(&source, &size, crop, &[]);
        assert_eq!(full.dimensions(), (4, 4));
        assert!(full.pixels().all(|p| p.0 == [0, 0, 255, 255]));
        let preview = imageops::resize(&source, 4, 2, imageops::FilterType::Nearest);
        assert_eq!(
            prepare_source(&preview, &size, crop, &[]).get_pixel(0, 0).0,
            [0, 0, 255, 255]
        );
    }
    #[test]
    fn effect_order_disable_and_transparency_are_observable() {
        let source = RgbImage::from_pixel(4, 4, Rgb([100, 50, 0]));
        let size = Resolution {
            width: 4,
            height: 4,
        };
        let effect = |kind, amount| LayerEffect {
            id: acheflow_media_api::workspace::new_entity_id(),
            shader: String::new(),
            kind,
            amount,
            enabled: true,
        };
        let effects = [
            effect(EffectKind::Brightness, 200),
            effect(EffectKind::Grayscale, 100),
            effect(EffectKind::Opacity, 50),
        ];
        let layer = prepare_source(&source, &size, SourceCrop::default(), &effects);
        let pixel = layer.get_pixel(0, 0);
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
        assert_eq!(pixel[3], 128);
        let mut canvas = RgbImage::from_pixel(4, 4, Rgb([0, 0, 255]));
        composite(&mut canvas, &layer, 0, 0);
        assert!(canvas.get_pixel(0, 0)[2] > canvas.get_pixel(0, 0)[0]);
        let mut first = vec![
            effect(EffectKind::Brightness, 200),
            effect(EffectKind::Contrast, 0),
        ];
        let a = prepare_source(&source, &size, SourceCrop::default(), &first);
        first.reverse();
        let b = prepare_source(&source, &size, SourceCrop::default(), &first);
        assert_ne!(a, b);
        first.iter_mut().for_each(|e| e.enabled = false);
        assert_eq!(
            prepare_source(&source, &size, SourceCrop::default(), &first)
                .get_pixel(0, 0)
                .0,
            [100, 50, 0, 255]
        );
    }
    #[test]
    fn blur_spreads_an_impulse_but_zero_is_identity() {
        let mut source = RgbImage::new(9, 9);
        source.put_pixel(4, 4, Rgb([255, 255, 255]));
        let size = Resolution {
            width: 9,
            height: 9,
        };
        let layer = prepare_source(
            &source,
            &size,
            SourceCrop::default(),
            &[LayerEffect {
                id: acheflow_media_api::workspace::new_entity_id(),
                shader: String::new(),
                kind: EffectKind::Blur,
                amount: 2,
                enabled: true,
            }],
        );
        assert!(layer.get_pixel(4, 4)[0] < 255);
        assert!(layer.get_pixel(3, 4)[0] > 0);
    }
}
