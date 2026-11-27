//! Bevy [`AssetLoader`] that turns `.art` sprites into [`Image`]s.

use arcanum_formats::art::{ArtError, ArtFile};
use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages, io::Reader};
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Decodes frame 0 of rotation 0 using the file's first palette.
#[derive(Default, TypePath)]
pub struct ArtAssetLoader;

#[derive(Debug)]
pub enum ArtLoadError {
    Io(std::io::Error),
    Art(ArtError),
}

impl std::fmt::Display for ArtLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "could not read .art file: {e}"),
            Self::Art(e) => write!(f, "invalid .art file: {e}"),
        }
    }
}

impl std::error::Error for ArtLoadError {}

impl AssetLoader for ArtAssetLoader {
    type Asset = Image;
    type Settings = ();
    type Error = ArtLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Image, ArtLoadError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(ArtLoadError::Io)?;
        let (rgba, (width, height)) = ArtFile::parse(&bytes)
            .and_then(|art| art.decode_frame0())
            .map_err(ArtLoadError::Art)?;
        Ok(rgba_image(width, height, rgba))
    }

    fn extensions(&self) -> &[&str] {
        // Extension matching is case-sensitive and the game mixes `.art` and `.ART`.
        &["art", "ART", "Art"]
    }
}

fn rgba_image(width: u32, height: u32, rgba: Vec<u8>) -> Image {
    Image::new(
        Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Every frame of an `.art` file, each as its own [`Image`]: rotation 0's labelled `frame0`,
/// `frame1`, ..., the other rotations' `r1frame0`, .... Used for multi-state art such as buttons
/// and for world objects.
#[derive(Asset, TypePath, Debug, Clone)]
pub struct ArtFrames {
    pub frames: Vec<Handle<Image>>,
    /// Each frame's hotspot, in pixels from its top-left corner: the point the engine places on
    /// the art's position.
    pub centers: Vec<IVec2>,
    /// Each frame's size in pixels.
    pub sizes: Vec<UVec2>,
    /// Playback speed from the file, in frames per second.
    pub frame_rate: u32,
    /// The frame a swing lands on (art header +0x1c, read by 0x5030b0; 0 none).
    pub action_frame: u32,
    /// Every rotation in the file (1 or 8), rotation 0 first (the same as the fields above).
    pub rotations: Vec<ArtRotation>,
}

/// One rotation's frames, hotspots and sizes.
#[derive(Debug, Clone)]
pub struct ArtRotation {
    pub frames: Vec<Handle<Image>>,
    pub centers: Vec<IVec2>,
    pub sizes: Vec<UVec2>,
}

impl ArtRotation {
    /// Frame `index` (the last frame if there are fewer) with its hotspot and size.
    pub fn frame(&self, index: usize) -> Option<(&Handle<Image>, IVec2, UVec2)> {
        let i = index.min(self.frames.len().checked_sub(1)?);
        Some((&self.frames[i], *self.centers.get(i)?, *self.sizes.get(i)?))
    }
}

impl ArtFrames {
    /// Frame `index`, or the last frame if there are fewer.
    pub fn frame(&self, index: usize) -> Option<&Handle<Image>> {
        self.frames.get(index).or(self.frames.last())
    }

    /// Rotation `index`, or rotation 0 when the file has only one.
    pub fn rotation(&self, index: usize) -> Option<&ArtRotation> {
        self.rotations.get(index).or(self.rotations.first())
    }

    /// The sprite anchor that puts frame `index`'s hotspot on the sprite's position (the last
    /// frame if there are fewer).
    pub fn anchor(&self, index: usize) -> bevy::sprite::Anchor {
        let i = index.min(self.frames.len().saturating_sub(1));
        let (Some(center), Some(size)) = (self.centers.get(i), self.sizes.get(i)) else { return bevy::sprite::Anchor::CENTER };
        hotspot_anchor(*center, *size)
    }
}

/// The anchor (fractions of the size from the middle, y up) for a hotspot at `center` pixels
/// from the top-left of a `size` image.
pub fn hotspot_anchor(center: IVec2, size: UVec2) -> bevy::sprite::Anchor {
    if size.x == 0 || size.y == 0 {
        return bevy::sprite::Anchor::CENTER;
    }
    bevy::sprite::Anchor(Vec2::new(center.x as f32 / size.x as f32 - 0.5, 0.5 - center.y as f32 / size.y as f32))
}

/// Loads `.art` files as [`ArtFrames`]. Shares the extension with the other `.art` loaders;
/// Bevy picks by the requested asset type.
#[derive(Default, TypePath)]
pub struct ArtFramesLoader;

impl AssetLoader for ArtFramesLoader {
    type Asset = ArtFrames;
    type Settings = ();
    type Error = ArtLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<ArtFrames, ArtLoadError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(ArtLoadError::Io)?;
        let art = ArtFile::parse(&bytes).map_err(ArtLoadError::Art)?;
        let palette = art.default_palette().map_err(ArtLoadError::Art)?;
        let mut rotations = Vec::with_capacity(art.rotation_count);
        for rotation in 0..art.rotation_count {
            let mut r = ArtRotation { frames: Vec::new(), centers: Vec::new(), sizes: Vec::new() };
            for index in 0..art.frame_count {
                let frame = art.decode_frame(rotation, index, palette).map_err(ArtLoadError::Art)?;
                r.centers.push(IVec2::new(frame.info.center_x, frame.info.center_y));
                r.sizes.push(UVec2::new(frame.info.width, frame.info.height));
                let image = rgba_image(frame.info.width, frame.info.height, frame.rgba);
                let label = if rotation == 0 { format!("frame{index}") } else { format!("r{rotation}frame{index}") };
                r.frames.push(load_context.add_labeled_asset(label, image));
            }
            rotations.push(r);
        }
        let Some(first) = rotations.first().filter(|r| !r.frames.is_empty()).cloned() else {
            return Err(ArtLoadError::Art(ArtError::NoFrames));
        };
        Ok(ArtFrames { frames: first.frames, centers: first.centers, sizes: first.sizes, frame_rate: art.frame_rate, action_frame: art.action_frame, rotations })
    }

    fn extensions(&self) -> &[&str] {
        // Extension matching is case-sensitive and the game mixes `.art` and `.ART`.
        &["art", "ART", "Art"]
    }
}
