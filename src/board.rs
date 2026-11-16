//! The board's own look: stickers placed on the canvas (built-in designs or
//! images) and its background (a preset, or an image). Saved with the
//! project; images travel inside it, so a project opens the same anywhere.
//!
//! Stickers can be PNG (with its transparency), JPEG or SVG; WebP is read
//! too. Raster images are kept encoded (PNG when they have transparency,
//! else JPEG), shrunk to a sensible size on import; SVG is kept as it is
//! and drawn at the size it is shown, so it stays sharp at any zoom. Every
//! sticker showing an image shares it.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Longest side of a sticker image, in pixels.
pub const STICKER_MAX_PX: u32 = 1024;
/// Longest side of a background image, in pixels.
pub const BACKGROUND_MAX_PX: u32 = 2560;
/// Width of a new sticker, in canvas units.
pub const STICKER_WIDTH: f32 = 150.0;
/// Narrowest and widest a sticker can be made.
pub const WIDTH_RANGE: (f32, f32) = (24.0, 2400.0);

/// What a sticker shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "lowercase")]
pub enum Art {
    /// One of the designs drawn by the app (see `ui::board::BUILTINS`).
    Builtin(String),
    /// An image of the board's, by id.
    Image(String),
}

/// A sticker on the board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sticker {
    pub id: u32,
    pub art: Art,
    /// Its centre, in canvas units.
    pub pos: [f32; 2],
    /// Its width, in canvas units (the height follows the art's shape).
    pub width: f32,
    /// Clockwise, in degrees.
    #[serde(default)]
    pub rotation: f32,
    /// Locked stickers ignore the pointer: the canvas under them is
    /// selected and panned as if they weren't there.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// What is behind the board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Background {
    /// A preset's id (see `ui::board::PRESETS`): its colour and grid.
    #[serde(default = "default_preset")]
    pub preset: String,
    /// An image of the board's, by id, filling the view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// How much the preset's colour veils the image (0: not at all), so
    /// nodes stay readable.
    #[serde(default = "default_dim")]
    pub dim: f32,
    #[serde(default = "yes")]
    pub grid: bool,
}

fn default_preset() -> String {
    "graphite".to_owned()
}

fn default_dim() -> f32 {
    0.55
}

fn yes() -> bool {
    true
}

impl Default for Background {
    fn default() -> Self {
        Self { preset: default_preset(), image: None, dim: default_dim(), grid: true }
    }
}

impl Background {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// An image: encoded (PNG or JPEG) with its size in pixels, or an SVG
/// document with its own size. Copies share the bytes; in files the bytes
/// are base64.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Blob {
    #[serde(with = "base64_bytes")]
    pub data: Arc<Vec<u8>>,
    pub w: u32,
    pub h: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub svg: bool,
}

impl PartialEq for Blob {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.data, &other.data) || self.data == other.data)
            && (self.w, self.h, self.svg) == (other.w, other.h, other.svg)
    }
}

mod base64_bytes {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&crate::project::base64_encode(data))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        let text = String::deserialize(d)?;
        crate::project::base64_decode(&text).map(Arc::new).ok_or_else(|| serde::de::Error::custom("bad base64 image"))
    }
}

/// The board's stickers, images and background.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Board {
    /// Back to front.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stickers: Vec<Sticker>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub images: BTreeMap<String, Blob>,
    #[serde(default, skip_serializing_if = "Background::is_default")]
    pub background: Background,
}

impl Board {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn sticker(&self, id: u32) -> Option<&Sticker> {
        self.stickers.iter().find(|s| s.id == id)
    }

    pub fn sticker_mut(&mut self, id: u32) -> Option<&mut Sticker> {
        self.stickers.iter_mut().find(|s| s.id == id)
    }

    /// Puts a sticker on top of the others; returns its id.
    pub fn add(&mut self, art: Art, pos: [f32; 2], width: f32) -> u32 {
        let id = self.stickers.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        let width = width.clamp(WIDTH_RANGE.0, WIDTH_RANGE.1);
        self.stickers.push(Sticker { id, art, pos, width, rotation: 0.0, locked: false });
        id
    }

    pub fn remove(&mut self, id: u32) {
        self.stickers.retain(|s| s.id != id);
        self.prune();
    }

    /// A copy, a little down and to the right, on top.
    pub fn duplicate(&mut self, id: u32) -> Option<u32> {
        let s = self.sticker(id)?.clone();
        let copy = self.add(s.art, [s.pos[0] + 24.0, s.pos[1] + 24.0], s.width);
        if let Some(c) = self.sticker_mut(copy) {
            c.rotation = s.rotation;
        }
        Some(copy)
    }

    /// In front of every other sticker (`front`) or behind them all.
    pub fn restack(&mut self, id: u32, front: bool) {
        if let Some(i) = self.stickers.iter().position(|s| s.id == id) {
            let s = self.stickers.remove(i);
            if front {
                self.stickers.push(s);
            } else {
                self.stickers.insert(0, s);
            }
        }
    }

    /// Drops images nothing shows any more.
    pub fn prune(&mut self) {
        let used = |id: &String| {
            self.background.image.as_ref() == Some(id) || self.stickers.iter().any(|s| s.art == Art::Image(id.clone()))
        };
        let keep: BTreeMap<String, Blob> = self.images.iter().filter(|(id, _)| used(id)).map(|(k, v)| (k.clone(), v.clone())).collect();
        self.images = keep;
    }

    /// Height over width of what a sticker shows (images: their own shape;
    /// designs: `builtin_aspect`).
    pub fn aspect(&self, art: &Art, builtin_aspect: impl Fn(&str) -> f32) -> f32 {
        match art {
            Art::Builtin(name) => builtin_aspect(name),
            Art::Image(id) => self.images.get(id).map_or(1.0, |b| b.h.max(1) as f32 / b.w.max(1) as f32),
        }
    }

    /// Takes an image file's bytes (PNG, JPEG, WebP shrunk to `max_px` on
    /// their longest side; SVG as it is); returns its id. The same image
    /// twice is kept once.
    pub fn import_image(&mut self, bytes: &[u8], max_px: u32, photo: bool) -> Result<String, String> {
        let blob = encode_image(bytes, max_px, photo)?;
        let id = format!("img{:016x}", fnv1a(&blob.data));
        self.images.entry(id.clone()).or_insert(blob);
        Ok(id)
    }
}

/// Biggest SVG taken, in bytes.
const SVG_MAX_BYTES: usize = 8 << 20;

/// Decodes, shrinks and re-encodes an image: PNG (keeps transparency) or,
/// for an opaque `photo`, JPEG. An SVG is checked and kept as it is.
pub fn encode_image(bytes: &[u8], max_px: u32, photo: bool) -> Result<Blob, String> {
    use image::{DynamicImage, ImageFormat};
    if looks_like_svg(bytes) {
        if bytes.len() > SVG_MAX_BYTES {
            return Err("this SVG is too large (over 8 MB)".to_owned());
        }
        let tree = svg_tree(bytes)?;
        let size = tree.size();
        return Ok(Blob { data: Arc::new(bytes.to_vec()), w: size.width().ceil() as u32, h: size.height().ceil() as u32, svg: true });
    }
    let img = image::load_from_memory(bytes).map_err(|e| format!("not an image this app can read ({e})"))?;
    let img = if img.width().max(img.height()) > max_px {
        img.resize(max_px, max_px, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let (w, h) = (img.width(), img.height());
    let opaque = !img.color().has_alpha() || img.to_rgba8().pixels().all(|p| p[3] == 255);
    let mut out = std::io::Cursor::new(Vec::new());
    if photo && opaque {
        let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 86);
        enc.encode_image(&rgb).map_err(|e| e.to_string())?;
    } else {
        DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut out, ImageFormat::Png).map_err(|e| e.to_string())?;
    }
    Ok(Blob { data: Arc::new(out.into_inner()), w, h, svg: false })
}

/// An SVG document (possibly gzipped, as .svgz), by its first bytes.
fn looks_like_svg(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0x1F, 0x8B]) {
        return true;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || ((head.starts_with("<?xml") || head.starts_with("<!--") || head.starts_with("<!doctype")) && head.contains("<svg"))
}

/// How SVGs are read. The system's fonts (for SVG text) are loaded once,
/// and only for an SVG that has text: loading them takes a moment.
fn svg_options(text: bool) -> &'static resvg::usvg::Options<'static> {
    static PLAIN: std::sync::OnceLock<resvg::usvg::Options<'static>> = std::sync::OnceLock::new();
    static FONTS: std::sync::OnceLock<resvg::usvg::Options<'static>> = std::sync::OnceLock::new();
    if !text {
        return PLAIN.get_or_init(resvg::usvg::Options::default);
    }
    FONTS.get_or_init(|| {
        let mut opt = resvg::usvg::Options::default();
        opt.fontdb_mut().load_system_fonts();
        opt
    })
}

fn svg_tree(bytes: &[u8]) -> Result<resvg::usvg::Tree, String> {
    // Gzipped (.svgz) files are looked into once unpacked by usvg; they are
    // given fonts in case.
    let text = bytes.starts_with(&[0x1F, 0x8B]) || bytes.windows(5).any(|w| w == b"<text");
    resvg::usvg::Tree::from_data(bytes, svg_options(text)).map_err(|e| format!("not an SVG this app can read ({e})"))
}

/// RGBA pixels of an image (width, height, premultiplied alpha?, pixels).
/// An SVG is drawn `px` pixels on its longest side; a raster image at its
/// own size.
pub fn decode_rgba(blob: &Blob, px: u32) -> Result<(u32, u32, bool, Vec<u8>), String> {
    if blob.svg {
        let tree = svg_tree(&blob.data)?;
        let size = tree.size();
        let scale = px.max(1) as f32 / size.width().max(size.height()).max(1e-3);
        let (w, h) = ((size.width() * scale).ceil().max(1.0) as u32, (size.height() * scale).ceil().max(1.0) as u32);
        let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or("SVG too large to draw")?;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        return Ok((w, h, true, pixmap.take()));
    }
    let img = image::load_from_memory(&blob.data).map_err(|e| e.to_string())?.to_rgba8();
    Ok((img.width(), img.height(), false, img.into_raw()))
}

/// File extensions taken as images.
pub fn is_image_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| ["png", "jpg", "jpeg", "webp", "svg", "svgz"].contains(&x.to_ascii_lowercase().as_str()))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small PNG made here: `w`×`h`, red, half transparent if asked.
    fn png(w: u32, h: u32, alpha: u8) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([220, 40, 40, alpha]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn images_are_shrunk_shared_and_pruned() {
        let mut b = Board::default();
        let big = png(3000, 1500, 128);
        let id = b.import_image(&big, STICKER_MAX_PX, false).unwrap();
        let blob = &b.images[&id];
        assert_eq!((blob.w, blob.h), (1024, 512), "shrunk to 1024 on its longest side");
        assert!(blob.data.starts_with(b"\x89PNG"), "kept as PNG for its transparency");
        assert_eq!(b.import_image(&big, STICKER_MAX_PX, false).unwrap(), id, "the same image is kept once");
        let s = b.add(Art::Image(id.clone()), [0.0, 0.0], 200.0);
        let copy = b.duplicate(s).unwrap();
        assert_eq!(b.aspect(&Art::Image(id.clone()), |_| 1.0), 0.5);
        b.remove(s);
        assert!(b.images.contains_key(&id), "still shown by the copy");
        b.remove(copy);
        assert!(b.images.is_empty(), "dropped once nothing shows it");
        // An opaque photo behind the board becomes a JPEG.
        let photo = png(800, 600, 255);
        let bg = b.import_image(&photo, BACKGROUND_MAX_PX, true).unwrap();
        assert!(b.images[&bg].data.starts_with(&[0xFF, 0xD8]), "JPEG");
        assert!(b.import_image(b"not an image", 100, false).is_err());
    }

    #[test]
    fn svg_stickers_stay_vectors() {
        let svg = br##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
  <circle cx="50" cy="50" r="40" fill="#e05a8a"/>
</svg>"##;
        let mut b = Board::default();
        let id = b.import_image(svg, STICKER_MAX_PX, false).unwrap();
        let blob = b.images[&id].clone();
        assert!(blob.svg && (blob.w, blob.h) == (200, 100));
        assert_eq!(&blob.data[..], &svg[..], "kept as written");
        // Drawn sharp at any size: 800 pixels wide here.
        let (w, h, premultiplied, px) = decode_rgba(&blob, 800).unwrap();
        assert_eq!((w, h), (800, 400));
        assert!(premultiplied);
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..4];
        assert!(at(200, 200)[3] == 255 && at(200, 200)[0] > 200, "inside the circle: opaque pink");
        assert_eq!(at(700, 200)[3], 0, "outside it: transparent");
        let json = serde_json::to_string(&b).unwrap();
        let back: Board = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
        assert!(b.import_image(b"<svg><g></svg", 100, false).is_err(), "broken SVG refused");
    }

    /// The board travels with the project: saved, and back after opening;
    /// a new project starts plain.
    #[test]
    fn boards_are_saved_with_the_project() {
        use std::sync::Arc as StdArc;
        let (a, _engine) = crate::audio::offline(StdArc::new(crate::audio::params::EngineShared::new()), 48_000);
        let mut g = crate::graph::Graph::default();
        let id = g.board.import_image(&png(64, 64, 100), STICKER_MAX_PX, false).unwrap();
        g.board.add(Art::Image(id), [50.0, 60.0], 140.0);
        g.board.add(Art::Builtin("cassette".into()), [300.0, 60.0], 150.0);
        g.board.background.preset = "moss".into();
        let saved = crate::project::snapshot(&g);
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains("\"board\"") && json.contains("cassette") && json.contains("moss"));
        let reread: crate::project::Project = serde_json::from_str(&json).unwrap();
        g.clear(&a);
        assert!(g.board.is_default(), "a new project starts plain");
        crate::project::restore(&mut g, &reread, &a);
        assert_eq!(g.board.stickers.len(), 2);
        assert_eq!(g.board.background.preset, "moss");
        assert_eq!(g.board.images.len(), 1);
        // A project without a board says nothing about it.
        let plain = crate::project::snapshot(&crate::graph::Graph::default());
        assert!(!serde_json::to_string(&plain).unwrap().contains("\"board\""));
    }

    #[test]
    fn boards_round_trip_through_json() {
        let mut b = Board::default();
        let id = b.import_image(&png(40, 20, 255), STICKER_MAX_PX, false).unwrap();
        b.add(Art::Image(id), [10.0, 20.0], 120.0);
        let wave = b.add(Art::Builtin("wave".into()), [300.0, 40.0], 150.0);
        b.sticker_mut(wave).unwrap().rotation = -12.0;
        b.restack(wave, false);
        assert_eq!(b.stickers[0].id, wave, "sent to the back");
        b.background = Background { preset: "blueprint".into(), image: None, dim: 0.4, grid: false };
        let json = serde_json::to_string(&b).unwrap();
        assert!(json.contains("\"kind\":\"builtin\"") && json.contains("blueprint"));
        let back: Board = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
        // An empty board writes nothing, and reads back from nothing.
        assert!(Board::default().is_default());
        let none: Board = serde_json::from_str("{}").unwrap();
        assert!(none.is_default());
        assert_eq!(serde_json::to_string(&Board::default()).unwrap(), "{}");
    }
}
