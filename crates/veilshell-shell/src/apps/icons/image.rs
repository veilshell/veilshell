use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use resvg::{tiny_skia, usvg};
use veilshell_render::RgbaImage;
use zune_jpeg::{
    zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions},
    JpegDecoder,
};

use super::{xdg, Format, PIXELS};

const MAX_BITMAP_SIDE: usize = 2048;
const MAX_BITMAP_BYTES: u64 = 24 * 1024 * 1024;

pub(super) fn load(path: &Path, format: Format) -> Option<RgbaImage> {
    let image = match format {
        Format::Png => load_bitmap(path)?,
        Format::Svg => load_svg(path)?,
    };
    // A corrupt/unsupported or empty icon should not hide a usable fallback.
    image
        .rgba
        .chunks_exact(4)
        .any(|px| px[3] != 0)
        .then_some(image)
}

fn load_bitmap(path: &Path) -> Option<RgbaImage> {
    let mut reader = BufReader::new(xdg::open_bounded(path, MAX_BITMAP_BYTES)?);
    // Some menu generators install JPEG bytes at a `.png` path. Keep themed
    // lookup unchanged and only enable the compatibility decoder for JPEG magic.
    let image = if reader.fill_buf().ok()?.starts_with(&[0xff, 0xd8, 0xff]) {
        let mut bytes = Vec::new();
        reader
            .take(MAX_BITMAP_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_BITMAP_BYTES {
            return None;
        }
        load_jpeg(&bytes)?
    } else {
        load_png(reader)?
    };
    if image.width <= PIXELS && image.height <= PIXELS {
        Some(image)
    } else {
        Some(image.resize_fit(PIXELS, PIXELS))
    }
}

fn load_png(reader: BufReader<File>) -> Option<RgbaImage> {
    let mut decoder = png::Decoder::new(reader);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let header = decoder.read_header_info().ok()?;
    // Reject huge sources before allocating the decoder's output buffer.
    if header.width as usize > MAX_BITMAP_SIDE || header.height as usize > MAX_BITMAP_SIDE {
        return None;
    }
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf.into_iter().flat_map(|g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|px| [px[0], px[0], px[0], px[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    RgbaImage::new(info.width, info.height, rgba)
}

fn load_jpeg(bytes: &[u8]) -> Option<RgbaImage> {
    let options = DecoderOptions::new_safe()
        .set_max_width(MAX_BITMAP_SIDE)
        .set_max_height(MAX_BITMAP_SIDE)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().ok()?;
    let (width, height) = decoder.dimensions()?;
    if decoder.output_colorspace()? != ColorSpace::RGBA {
        return None;
    }
    RgbaImage::new(
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        decoder.decode().ok()?,
    )
}

fn load_svg(path: &Path) -> Option<RgbaImage> {
    let data = xdg::read_bytes(path, 2 * 1024 * 1024)?;
    let options = usvg::Options {
        // Desktop icons should be self-contained. Don't follow SVG references
        // to other files or network resources.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            ..Default::default()
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(&data, &options).ok()?;
    let size = tree.size();
    let scale = PIXELS as f32 / size.width().max(size.height());
    let width = (size.width() * scale).round().clamp(1.0, PIXELS as f32) as u32;
    let height = (size.height() * scale).round().clamp(1.0, PIXELS as f32) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    RgbaImage::new(width, height, pixmap.take_demultiplied())
}
