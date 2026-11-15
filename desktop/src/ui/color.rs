//! `color-mix(in oklch, ...)` の再現。Web 版の secondary ボタンの hover が使う。

use gpui::Rgba;

use crate::tokens::oklch_to_srgb;

fn to_linear(v: f32) -> f64 {
    let v = v as f64;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB を OKLCH `(L, C, H度)` にする。
pub fn srgb_to_oklch(c: Rgba) -> (f32, f32, f32) {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let lab_l = 0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s;
    let a = 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s;
    let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s;
    let chroma = a.hypot(bb);
    let hue = bb.atan2(a).to_degrees().rem_euclid(360.0);
    (lab_l as f32, chroma as f32, hue as f32)
}

/// 彩度がこれ未満なら色相は無いものとして扱う (CSS の powerless)。
const POWERLESS: f32 = 0.000_4;

/// `color-mix(in oklch, a, b p%)`。`p` は 0〜1 で、`b` の割合。色相は短い側を回る。
pub fn mix_oklch(a: Rgba, b: Rgba, p: f32) -> Rgba {
    let (la, ca, ha) = srgb_to_oklch(a);
    let (lb, cb, hb) = srgb_to_oklch(b);
    let l = la + (lb - la) * p;
    let c = ca + (cb - ca) * p;
    let h = match (ca < POWERLESS, cb < POWERLESS) {
        (true, true) => 0.0,
        (true, false) => hb,
        (false, true) => ha,
        (false, false) => {
            let mut d = hb - ha;
            if d > 180.0 {
                d -= 360.0;
            } else if d < -180.0 {
                d += 360.0;
            }
            (ha + d * p).rem_euclid(360.0)
        }
    };
    oklch_to_srgb(l, c, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::{hex, to_u8};

    #[test]
    fn roundtrips_through_oklch() {
        for v in [0xffd600, 0x1d4ed8, 0xf5f5f5, 0x626262, 0x000000, 0xffffff] {
            let (l, c, h) = srgb_to_oklch(hex(v));
            assert_eq!(to_u8(oklch_to_srgb(l, c, h)), to_u8(hex(v)), "{v:06x}");
        }
    }

    #[test]
    fn mixing_with_zero_or_full_returns_ends() {
        let a = hex(0xf5f5f5);
        let b = hex(0x000000);
        assert_eq!(to_u8(mix_oklch(a, b, 0.0)), to_u8(a));
        assert_eq!(to_u8(mix_oklch(a, b, 1.0)), to_u8(b));
    }

    #[test]
    fn secondary_hover_is_slightly_darker() {
        // #f5f5f5 に黒を 5% 混ぜる。OKLab では L: 0.97 -> 0.9215 付近になる。
        let mixed = to_u8(mix_oklch(hex(0xf5f5f5), hex(0x000000), 0.05));
        assert!(mixed[0] < 0xf5 && mixed[0] > 0xe0, "{mixed:?}");
        assert_eq!(mixed[0], mixed[1]);
        assert_eq!(mixed[1], mixed[2]);
    }

    #[test]
    fn hue_takes_the_short_way() {
        let a = oklch_to_srgb(0.7, 0.1, 350.0);
        let b = oklch_to_srgb(0.7, 0.1, 10.0);
        let (_, _, h) = srgb_to_oklch(mix_oklch(a, b, 0.5));
        assert!(!(20.0..340.0).contains(&h), "{h}");
    }
}
