//! Colours as the saved palette and its sources spell them.
//!
//! Eight-digit hex follows Qt's `#AARRGGBB`: alpha first. `#e00e1e36` is the
//! colour `#0e1e36` at alpha `0xe0`, never CSS's `RRGGBBAA`. `Qt.rgba(r, g,
//! b, a)` takes normalised components, written as decimals or simple
//! fractions such as `14 / 255`.

use std::fmt;

/// An 8-bit-per-channel colour with straight (non-premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Parse `#rgb`, `#rrggbb`, `#aarrggbb` (alpha first, as Qt) or
    /// `Qt.rgba(...)`. Surrounding whitespace and quotes are ignored.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        if let Some(args) = text
            .strip_prefix("Qt.rgba(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            return Self::parse_qt_rgba(args);
        }
        Self::parse_hex(text)
    }

    fn parse_hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#').unwrap_or(text);
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        match digits.len() {
            3 => {
                let nibble = |at: usize| {
                    u8::from_str_radix(&digits[at..=at], 16).ok().map(|v| v * 17)
                };
                Some(Self::rgb(nibble(0)?, nibble(1)?, nibble(2)?))
            }
            6 => Some(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            // Alpha FIRST: the RGB channels are never shifted by it.
            8 => Some(Self {
                a: byte(0)?,
                r: byte(2)?,
                g: byte(4)?,
                b: byte(6)?,
            }),
            _ => None,
        }
    }

    fn parse_qt_rgba(args: &str) -> Option<Self> {
        let mut parts = args.split(',');
        let mut next = || parts.next().and_then(component);
        let (r, g, b, a) = (next()?, next()?, next()?, next()?);
        if parts.next().is_some() {
            return None;
        }
        let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(Self {
            r: byte(r),
            g: byte(g),
            b: byte(b),
            a: byte(a),
        })
    }

    /// `#rrggbb`: the colour with alpha discarded, channels untouched.
    pub fn rgb_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// The saved spelling: `#rrggbb` when opaque, else Qt's `#aarrggbb`.
    pub fn to_saved(self) -> String {
        if self.a == 255 {
            self.rgb_hex()
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.a, self.r, self.g, self.b)
        }
    }

    pub const fn opaque(self) -> Self {
        Self { a: 255, ..self }
    }

    /// Linear mix of the channels (alpha included) toward `other`.
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let blend = |from: u8, to: u8| {
            (f32::from(from) + (f32::from(to) - f32::from(from)) * amount)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Self {
            r: blend(self.r, other.r),
            g: blend(self.g, other.g),
            b: blend(self.b, other.b),
            a: blend(self.a, other.a),
        }
    }

    /// WCAG relative luminance of the RGB channels.
    pub fn luminance(self) -> f32 {
        let linear = |channel: u8| {
            let value = f32::from(channel) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    pub fn is_light(self) -> bool {
        self.luminance() > 0.4
    }

    /// Whichever of black or white reads better on this colour.
    pub fn readable_on(self) -> Self {
        if self.luminance() > 0.179 {
            Self::rgb(0x10, 0x10, 0x10)
        } else {
            Self::rgb(0xf8, 0xf8, 0xf8)
        }
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_saved())
    }
}

/// A number or a simple `a / b` fraction.
fn component(text: &str) -> Option<f64> {
    let text = text.trim();
    match text.split_once('/') {
        Some((top, bottom)) => {
            let bottom: f64 = bottom.trim().parse().ok()?;
            (bottom != 0.0).then(|| top.trim().parse::<f64>().ok().map(|t| t / bottom))?
        }
        None => text.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qt_hex_puts_alpha_first_and_keeps_rgb_in_place() {
        let color = Rgba::parse("#e00e1e36").unwrap();
        assert_eq!(color, Rgba { r: 0x0e, g: 0x1e, b: 0x36, a: 0xe0 });
        assert_eq!(color.rgb_hex(), "#0e1e36");
        assert_eq!(color.to_saved(), "#e00e1e36");
    }

    #[test]
    fn qt_rgba_accepts_fractions_and_decimals() {
        let color =
            Rgba::parse("Qt.rgba(14 / 255, 30 / 255, 54 / 255, 0.88)").unwrap();
        assert_eq!((color.r, color.g, color.b), (14, 30, 54));
        assert_eq!(color.a, 224);
        assert_eq!(Rgba::parse("Qt.rgba(1,0,0.5,1)").unwrap(), Rgba { r: 255, g: 0, b: 128, a: 255 });
    }

    #[test]
    fn plain_forms_and_garbage() {
        assert_eq!(Rgba::parse("\"#0E1E36\"").unwrap(), Rgba::rgb(14, 30, 54));
        assert_eq!(Rgba::parse("0e1e36").unwrap(), Rgba::rgb(14, 30, 54));
        assert_eq!(Rgba::parse("#fff").unwrap(), Rgba::rgb(255, 255, 255));
        assert!(Rgba::parse("#12345").is_none());
        assert!(Rgba::parse("#gggggg").is_none());
        assert!(Rgba::parse("Qt.rgba(1, 2, 3)").is_none());
        assert!(Rgba::parse("Qt.rgba(1/0, 0, 0, 1)").is_none());
    }

    #[test]
    fn opaque_colours_save_without_alpha() {
        assert_eq!(Rgba::rgb(1, 2, 3).to_saved(), "#010203");
        assert_eq!(Rgba { a: 0, ..Rgba::rgb(1, 2, 3) }.to_saved(), "#00010203");
    }
}
