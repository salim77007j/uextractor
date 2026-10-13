//! X11 root-window capture for automated screenshots (autoshot feature).

use image::codecs::png::PngEncoder;
use image::{ImageBuffer, Rgba};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub fn capture_root_window() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let geom = conn.get_geometry(root)?.reply()?;
    let img = conn.get_image(
        ImageFormat::Z_PIXMAP,
        root,
        0,
        0,
        geom.width,
        geom.height,
        !0,
    )?
    .reply()?;

    // X11 ZPixmap with 24/32bpp = BGRx in memory
    let w = geom.width as u32;
    let h = geom.height as u32;
    let data = &img.data;
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    for px in data.chunks_exact(4) {
        buf.push(px[2]); // R
        buf.push(px[1]); // G
        buf.push(px[0]); // B
        buf.push(255); // A
    }
    let img_buf: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(w, h, buf).ok_or("bad buffer size")?;

    let mut out = std::io::Cursor::new(Vec::new());
    use image::{ExtendedColorType, ImageEncoder};
    PngEncoder::new(&mut out).write_image(
        img_buf.as_raw(),
        w,
        h,
        ExtendedColorType::Rgba8,
    )?;
    Ok(out.into_inner())
}
