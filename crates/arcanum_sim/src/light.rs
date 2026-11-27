//! Ambient light (Arcanum.exe 0x4a8060 / 0x4a7eb0 / 0x4d8560; the luminance tables 0x52c9b0).
//!
//! - **Schemes** (`rules/Lighting Schemes.mes`, read at 0x4a8060 through `Rules\%s.mes`): each
//!   names a file of 24 hours, each `ro, go, bo, ri, gi, bi` (outdoor then indoor colour). An
//!   empty hour takes values joined linearly from the nearest given hours round it; the first
//!   and last are always given. Message 1 is the default.
//! - **The hour's ambient** (0x4a7eb0, hour 0-23 from 0x45aaa0): the outdoor colour goes to
//!   one ambient and the indoor one to the other (0x4d8560); a ground tile's outdoor flag (art
//!   bit 8, 0x5038c0) picks the outdoor colour ([`indoor_tile`]).
//! - **Luminance** (tables 0x739f18 / 0x739f30 / 0x739f20, built at 0x52c9b0-0x52cab0):
//!   `tabR[r] + tabG[g] + tabB[b]`, each `channel / 255 × 76.5`, `× 150.45`, `× 28.05`
//!   (doubles 0x5970d0 / c8 / c0), at most 254.
//! - **The dark's difficulty** (0x4c8a0d-0x4c8a2d): `(255 − L) × 30 / 255`.
//! - **Which scheme** (0x4f2361-0x4f238c): the sector's (version 3 and on, 0x4d1916), else the
//!   default, 1 (no map has a `mapinfo.txt` with `LightScheme`). In the game's maps, K'na Tha
//!   (2, 9), Stillwater Giants Cave (2), The Pit (9) and Tulla's Conveyance (10) use others.
//!
//! - **Lights** ([`crate::LoadedMap::light_at`], 0x4d89e0 with flag 1): on the ambient, each
//!   sector light not off (flag 0x1, 0x4d8b8f) whose art frame covers the sample point (the
//!   tile's centre, `loc_to_xy` + (0x28, 0x14)) adds its art pixel there, a colour-key pixel
//!   nothing (0x4d8bf1-0x4d8c27). The art is the falloff (no radius). A coloured light
//!   multiplies the pixel by its colour (the product tables 0x739ea4…; white uses the pixel
//!   as is, 0x4de46d); flags 0x8 / 0x10 take the outdoor / indoor ambient as that colour
//!   (0x4d8c30). Each channel adds, saturating at 255 (0x4d8e4c-0x4d8e93); flag 0x2 subtracts,
//!   down to 0 (0x4d8d96-0x4d8e45). The frame's top-left is the light's tile centre plus its
//!   pixel offset less the frame's centre (0x4de290 → 0x5031c0).
//!
//! Not built: light objects (lanterns, 0x4dc210 for objects' lit colour), animated light
//! frames (frame 0 here), the difficulty term itself in to-hit and prowling (it waits for the
//! light sources: an ambient-only night would darken fights beside torches).

/// The schemes' list.
pub const SCHEMES: &str = "rules/Lighting Schemes.mes";
/// The default scheme's number in [`SCHEMES`].
pub const DEFAULT_SCHEME: u32 = 1;

/// Every scheme of [`SCHEMES`] by number, read once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schemes(pub std::collections::BTreeMap<u32, Scheme>);

impl Schemes {
    /// Reads the list and each scheme it names (`rules/<name>.mes`); unreadable ones are left out.
    pub fn load(vfs: &arcanum_vfs::Vfs) -> Self {
        let read = |path: &str| vfs.read(path).ok().and_then(|d| arcanum_formats::mes::MesFile::parse(&d).ok());
        let Some(list) = read(SCHEMES) else { return Self::default() };
        Self(list.messages.iter().filter(|(n, _)| **n >= DEFAULT_SCHEME).filter_map(|(&n, name)| Some((n, Scheme::parse(&read(&format!("rules/{}.mes", name.trim()))?)?))).collect())
    }

    /// Scheme `number` (0: the default, 1), else the default.
    pub fn get(&self, number: u32) -> Option<&Scheme> {
        self.0.get(&number).or_else(|| self.0.get(&DEFAULT_SCHEME))
    }
}

/// Whether ground tile art `art` is lit by the indoor ambient: its outdoor flag, bit 8
/// (0x5038c0), clear. 0x4dc279 takes 0x602ea8 for the flag set; which file triple 0x4a7eb0
/// pairs with it was not settled (the parser 0x4a8060 not read): here the file's own order,
/// outdoor first (builder's call, as the game shows daylight outdoors).
pub fn indoor_tile(art: arcanum_formats::art_id::ArtId) -> bool {
    art.0 & 0x100 == 0
}

impl crate::LoadedMap {
    /// The ambient colour on tile `at` at `hour` (0x4d89e0 before its lights): the scheme of the
    /// tile's sector, indoor or outdoor by its ground art. `None` off the loaded sectors or
    /// without schemes.
    pub fn ambient_at(&self, at: arcanum_formats::map::TileLoc, hour: u32) -> Option<Rgb> {
        let sector = self.sectors.get(&at.sector())?;
        let (x, y) = at.tile_in_sector();
        let scheme = self.light.get(sector.light_scheme)?;
        Some(scheme.ambient(hour, indoor_tile(sector.tile(x, y).art_id())))
    }
}

/// The light flags (0x4d8b8f, 0x4d8d83, 0x4d8c30).
pub const LIGHT_OFF: u32 = 0x1;
pub const LIGHT_DARKENS: u32 = 0x2;
pub const LIGHT_OUTDOOR_COLOUR: u32 = 0x8;
pub const LIGHT_INDOOR_COLOUR: u32 = 0x10;

/// A tile's centre in the original's screen pixels (`loc_to_xy` 0x4b8680 + (0x28, 0x14)).
pub fn tile_pixel(at: arcanum_formats::map::TileLoc) -> (i64, i64) {
    let (x, y) = (i64::from(at.x), i64::from(at.y));
    (40 * (y - x), 20 * (x + y) + 20)
}

/// What `light` (art `frame`) adds at screen point `(px, py)`: its pixel there, tinted by
/// `tint` unless white; `None` off its frame or on a colour-key pixel.
pub fn light_pixel(light: &arcanum_formats::sector::Light, frame: &arcanum_formats::art::ArtFrame, tint: Rgb, (px, py): (i64, i64)) -> Option<Rgb> {
    let (cx, cy) = tile_pixel(arcanum_formats::map::TileLoc { x: light.x as u32, y: light.y as u32 });
    let left = cx + i64::from(light.offset_x) - i64::from(frame.info.center_x);
    let top = cy + i64::from(light.offset_y) - i64::from(frame.info.center_y);
    let (lx, ly) = (px - left, py - top);
    if lx < 0 || ly < 0 || lx >= i64::from(frame.info.width) || ly >= i64::from(frame.info.height) {
        return None;
    }
    let i = (ly as usize * frame.info.width as usize + lx as usize) * 4;
    let p = frame.rgba.get(i..i + 4)?;
    if p[3] == 0 {
        return None;
    }
    let mut c = [p[0], p[1], p[2]];
    if tint != [255, 255, 255] {
        for (ch, t) in c.iter_mut().zip(tint) {
            *ch = (u16::from(*ch) * u16::from(t) / 255) as u8;
        }
    }
    Some(c)
}

/// Adds (or for a darkening light takes away) `add` on `base`, each channel kept in 0-255.
pub fn combine(base: Rgb, add: Rgb, darkens: bool) -> Rgb {
    std::array::from_fn(|i| if darkens { base[i].saturating_sub(add[i]) } else { base[i].saturating_add(add[i]) })
}

impl crate::LoadedMap {
    /// Decodes frame 0 of each light's art in `sector` not read yet.
    pub(crate) fn read_light_frames(&mut self, vfs: &arcanum_vfs::Vfs, sector: &arcanum_formats::sector::Sector) {
        for light in &sector.lights {
            if self.light_frames.contains_key(&light.art_id.0) {
                continue;
            }
            let frame = self.art.as_ref().and_then(|a| a.path(light.art_id)).and_then(|p| vfs.read(&p).ok()).and_then(|d| arcanum_formats::art::ArtFile::parse(&d).ok()).and_then(|art| art.decode_frame(0, 0, art.default_palette().ok()?).ok());
            self.light_frames.insert(light.art_id.0, frame.map(std::sync::Arc::new));
        }
    }

    /// The light on tile `at` at `hour` (0x4d9240 → 0x4d89e0): its ambient ([`Self::ambient_at`])
    /// and every loaded sector's lights that reach it.
    pub fn light_at(&self, at: arcanum_formats::map::TileLoc, hour: u32) -> Option<Rgb> {
        let mut colour = self.ambient_at(at, hour)?;
        let scheme = self.sectors.get(&at.sector()).and_then(|s| self.light.get(s.light_scheme));
        let point = tile_pixel(at);
        for light in self.sectors.values().flat_map(|s| &s.lights).filter(|l| l.flags & LIGHT_OFF == 0) {
            let Some(Some(frame)) = self.light_frames.get(&light.art_id.0) else { continue };
            let tint = match (light.flags & LIGHT_OUTDOOR_COLOUR != 0, light.flags & LIGHT_INDOOR_COLOUR != 0, scheme) {
                (true, _, Some(s)) => s.ambient(hour, false),
                (_, true, Some(s)) => s.ambient(hour, true),
                _ => light.color,
            };
            if let Some(add) = light_pixel(light, frame, tint, point) {
                colour = combine(colour, add, light.flags & LIGHT_DARKENS != 0);
            }
        }
        Some(colour)
    }
}

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

/// `critter_flags` of a critter bearing a light (0x467e80; names from the string table):
/// `OCF_LIGHT_SMALL` 0x80000, `_MEDIUM` 0x100000, `_LARGE` 0x200000, `_XLARGE` 0x400000.
pub const OCF_LIGHT_SMALL: i32 = 0x8_0000;
pub const OCF_LIGHT_MEDIUM: i32 = 0x10_0000;
pub const OCF_LIGHT_LARGE: i32 = 0x20_0000;
pub const OCF_LIGHT_XLARGE: i32 = 0x40_0000;
pub const OCF_LIGHTS: i32 = OCF_LIGHT_SMALL | OCF_LIGHT_MEDIUM | OCF_LIGHT_LARGE | OCF_LIGHT_XLARGE;
/// `item_flags` of a light-giving item, `OIF_LIGHT_SMALL` 0x200 to `_XLARGE` 0x1000.
pub const OIF_LIGHTS: i32 = 0x1e00;

/// The light a critter bears from its worn items (0x467e80, slots 1000-1008): each item's
/// `item_flags` 0x200-0x1000, shifted down by its condition (0x461540, `100 × hp / most`: at
/// most 25 by three, 50 by two, 75 by one, 0x467efe-0x467f17), all ORed; the largest bit wins
/// (0x467f55-0x467f7e). `worn` gives each item's flags and condition.
pub fn borne_light(worn: impl Iterator<Item = (i32, i32)>) -> i32 {
    let bits = worn.fold(0, |acc, (flags, condition)| {
        let shift = if condition <= 25 { 3 } else if condition <= 50 { 2 } else if condition <= 75 { 1 } else { 0 };
        acc | ((flags & OIF_LIGHTS) >> shift) & OIF_LIGHTS
    });
    if bits & 0x1000 != 0 {
        OCF_LIGHT_XLARGE
    } else if bits & 0x800 != 0 {
        OCF_LIGHT_LARGE
    } else if bits & 0x400 != 0 {
        OCF_LIGHT_MEDIUM
    } else if bits & 0x200 != 0 {
        OCF_LIGHT_SMALL
    } else {
        0
    }
}

/// The dark's term `p` for a bearer of light `flags` (0x4c8a2f-0x4c8a7c): a quarter with an
/// extra-large light, a third large, half medium, three quarters small.
pub fn borne_scale(p: i32, flags: i32) -> i32 {
    if flags & OCF_LIGHT_XLARGE != 0 {
        p / 4
    } else if flags & OCF_LIGHT_LARGE != 0 {
        p / 3
    } else if flags & OCF_LIGHT_MEDIUM != 0 {
        p / 2
    } else if flags & OCF_LIGHT_SMALL != 0 {
        p * 3 / 4
    } else {
        p
    }
}

impl crate::Inventory {
    /// The light its wearer bears ([`borne_light`]).
    pub fn borne_light(&self, prototypes: &arcanum_formats::object::Prototypes) -> i32 {
        borne_light(self.items.iter().filter(|i| (crate::FIRST_SLOT..=crate::LAST_SLOT).contains(&i.location)).filter_map(|i| {
            let o = prototypes.get(i.prototype)?;
            let most = crate::repair::item_max_hp(i, prototypes);
            let condition = if most > 0 { 100 * (most - i.wear.damage) / most } else { 100 };
            Some((prototypes.int(o, "item_flags").unwrap_or(0), condition))
        }))
    }
}

/// What the dark adds to a PC's Pick Locks (`spared_from` Expert, 2: 0x4c8954 shares melee's
/// test), Spot Trap or Disarm Traps (from Apprentice, 1: 0x4c8982) roll of training `rank` at
/// light `l` ([`dark_difficulty`]); `None` light (no schemes) nothing.
pub fn skill_dark(rank: u8, spared_from: u8, l: Option<i32>) -> i32 {
    match l {
        Some(l) if rank < spared_from => dark_difficulty(l),
        _ => 0,
    }
}

impl crate::LoadedMap {
    /// The luminance on tile `at` at game time `now` ([`Self::light_at`]).
    pub fn luminance_at(&self, at: arcanum_formats::map::TileLoc, now: crate::GameTime) -> Option<i32> {
        self.light_at(at, now.ms / crate::HOUR_MS).map(luminance)
    }
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
        // Borne light (0x467e80): the largest wins; a worn-out lantern gives less.
        assert_eq!(borne_light([(0x1000, 100), (0x200, 100)].into_iter()), OCF_LIGHT_XLARGE);
        assert_eq!((borne_light([(0x1000, 60)].into_iter()), borne_light([(0x1000, 20)].into_iter()), borne_light([(0x200, 20)].into_iter())), (OCF_LIGHT_LARGE, OCF_LIGHT_SMALL, 0));
        assert_eq!((borne_scale(30, OCF_LIGHT_XLARGE), borne_scale(30, OCF_LIGHT_LARGE), borne_scale(30, OCF_LIGHT_MEDIUM), borne_scale(30, OCF_LIGHT_SMALL), borne_scale(30, 0)), (7, 10, 15, 22, 30));
        assert_eq!((skill_dark(0, 1, Some(79)), skill_dark(1, 1, Some(0)), skill_dark(1, 2, Some(0)), skill_dark(2, 2, Some(0)), skill_dark(0, 1, None)), (20, 0, 30, 0, 0), "spared from the rank given; no light data, nothing");
        // A 3 x 3 light frame centred on its tile, its middle pixel 100 grey, a colour key around.
        let mut rgba = vec![0u8; 36];
        rgba[16..20].copy_from_slice(&[100, 100, 100, 255]);
        let info = arcanum_formats::art::FrameInfo { width: 3, height: 3, data_size: 0, center_x: 1, center_y: 1, delta_x: 0, delta_y: 0 };
        let frame = arcanum_formats::art::ArtFrame { info, rgba };
        let at = arcanum_formats::map::TileLoc { x: 10, y: 10 };
        let light = arcanum_formats::sector::Light { x: 10, y: 10, ..Default::default() };
        assert_eq!(light_pixel(&light, &frame, [255, 255, 255], tile_pixel(at)), Some([100, 100, 100]), "white: the pixel");
        assert_eq!(light_pixel(&light, &frame, [255, 0, 128], tile_pixel(at)), Some([100, 0, 50]), "tinted");
        assert_eq!(light_pixel(&light, &frame, [255; 3], tile_pixel(arcanum_formats::map::TileLoc { x: 11, y: 10 })), None, "off its frame");
        let (px, py) = tile_pixel(at);
        assert_eq!(light_pixel(&light, &frame, [255; 3], (px + 1, py)), None, "a colour-key pixel");
        assert_eq!((combine([200, 10, 0], [100, 100, 100], false), combine([200, 10, 0], [100, 100, 100], true)), ([255, 110, 100], [100, 0, 0]));
        let open = arcanum_formats::mes::MesFile::parse(b"{0}{}{23}{1, 1, 1, 1, 1, 1}").unwrap();
        assert!(Scheme::parse(&open).is_none(), "the first hour must be given");
    }
}
