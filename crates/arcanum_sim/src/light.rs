//! Ambient light (Arcanum.exe 0x4a8060 / 0x4a7eb0 / 0x4d8560; the luminance tables 0x52c9b0).
//!
//! - **Schemes** (`rules/Lighting Schemes.mes`, read at 0x4a8060 through `Rules\%s.mes`): each
//!   names a file of 24 hours, each `ro, go, bo, ri, gi, bi` (outdoor then indoor colour). An
//!   empty hour takes values joined linearly from the nearest given hours round it; the first
//!   and last are always given. Message 1 is the default.
//! - **The hour's ambient** (0x4a7eb0, hour 0-23 from 0x45aaa0): the outdoor colour goes to
//!   0x603408, the indoor one to 0x602ea8. A ground tile whose art has bit 8 (0x5038c0) is lit by
//!   the indoor colour (0x4dc279).
//! - **Luminance** (tables 0x739f18 / 0x739f30 / 0x739f20, built at 0x52c9b0-0x52cab0):
//!   `tabR[r] + tabG[g] + tabB[b]`, each `channel / 255 × 76.5`, `× 150.45`, `× 28.05`
//!   (doubles 0x5970d0 / c8 / c0), at most 254.
//! - **The dark's difficulty** (0x4c8a0d-0x4c8a2d): `(255 − L) × 30 / 255`.
//!
//! Not built: light sources (sector lights, light objects: 0x4dc210 / 0x4d89e0 not read),
//! which scheme a map uses, the difficulty term itself in to-hit and prowling (it waits for the
//! light sources: an ambient-only night would darken fights beside torches).

/// The schemes' list.
pub const SCHEMES: &str = "rules/Lighting Schemes.mes";
/// The default scheme's number in [`SCHEMES`].
pub const DEFAULT_SCHEME: u32 = 1;

/// An RGB colour.
pub type Rgb = [u8; 3];

/// One scheme: each hour's outdoor and indoor colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheme {
    pub hours: [(Rgb, Rgb); 24],
}

impl Scheme {
    /// Parses a scheme file: messages 0-23, six numbers each, empty ones joined linearly from
    /// the nearest given hours (0x4a8060). `None` when the first or last hour is empty or a
    /// line does not read.
    pub fn parse(mes: &arcanum_formats::mes::MesFile) -> Option<Self> {
        let mut given: Vec<Option<[i32; 6]>> = Vec::with_capacity(24);
        for hour in 0..24 {
            let text = mes.get(hour).unwrap_or("").trim();
            if text.is_empty() {
                given.push(None);
                continue;
            }
            let values: Vec<i32> = text.split(',').map(|v| v.trim().parse().ok()).collect::<Option<_>>()?;
            given.push(Some(values.try_into().ok()?));
        }
        given[0]?;
        given[23]?;
        let mut hours = [([0; 3], [0; 3]); 24];
        for (hour, slot) in hours.iter_mut().enumerate() {
            let values = match given[hour] {
                Some(v) => v,
                None => {
                    let before = (0..hour).rev().find(|&h| given[h].is_some())?;
                    let after = (hour + 1..24).find(|&h| given[h].is_some())?;
                    let (a, b) = (given[before]?, given[after]?);
                    let (t, span) = ((hour - before) as i32, (after - before) as i32);
                    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t / span)
                }
            };
            let c = |i: usize| values[i].clamp(0, 255) as u8;
            *slot = ([c(0), c(1), c(2)], [c(3), c(4), c(5)]);
        }
        Some(Self { hours })
    }

    /// The ambient colour at `hour` (0-23), indoors or out (0x4a7eb0).
    pub fn ambient(&self, hour: u32, indoor: bool) -> Rgb {
        let (outdoor, inside) = self.hours[(hour % 24) as usize];
        if indoor { inside } else { outdoor }
    }
}

/// A colour's luminance as the original weighs it (0x739f18 / 0x739f30 / 0x739f20): each
/// channel's table entry, `channel / 255 × weight` cut to a whole number, summed.
pub fn luminance([r, g, b]: Rgb) -> i32 {
    let entry = |c: u8, weight: f64| (f64::from(c) / 255.0 * weight) as i32;
    entry(r, 76.5) + entry(g, 150.45) + entry(b, 28.05)
}

/// What the dark adds to a skill's difficulty at light `l` (0x4c8a0d-0x4c8a2d): `(255 − L) ×
/// 15 × 2 / 255`.
pub fn dark_difficulty(l: i32) -> i32 {
    (255 - l.clamp(0, 255)) * 15 * 2 / 255
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hours_are_joined_and_light_weighs_as_the_original() {
        let mes = arcanum_formats::mes::MesFile::parse(b"{0}{0, 0, 0, 10, 10, 10}{1}{}{2}{}{3}{30, 60, 90, 10, 10, 10}{4}{}{5}{}{6}{}{7}{}{8}{}{9}{}{10}{}{11}{}{12}{}{13}{}{14}{}{15}{}{16}{}{17}{}{18}{}{19}{}{20}{}{21}{}{22}{}{23}{30, 60, 90, 10, 10, 10}").unwrap();
        let scheme = Scheme::parse(&mes).unwrap();
        assert_eq!(scheme.ambient(1, false), [10, 20, 30], "a third of the way to hour 3");
        assert_eq!(scheme.ambient(12, true), [10, 10, 10]);
        assert_eq!(luminance([255, 255, 255]), 254, "76 + 150 + 28");
        assert_eq!(luminance([74, 80, 98]), 22 + 47 + 10);
        assert_eq!((dark_difficulty(254), dark_difficulty(79), dark_difficulty(0)), (0, 20, 30));
        let open = arcanum_formats::mes::MesFile::parse(b"{0}{}{23}{1, 1, 1, 1, 1, 1}").unwrap();
        assert!(Scheme::parse(&open).is_none(), "the first hour must be given");
    }
}
