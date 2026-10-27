//! The tray icon, rendered from our SVG and sent as pixels. Linux tray hosts look icons up by
//! name inconsistently (and cache misses), so pixels are the reliable option.

use resvg::{tiny_skia, usvg};

const TRAY_SVG: &str = include_str!("../../../packaging/icons/x360truewireless-tray-symbolic.svg");
/// The colour the SVG's stylesheet uses for `currentColor`; replaced with the panel's text colour.
const SVG_TEXT_COLOR: &str = "#232629";
const SIZES: [u32; 5] = [16, 22, 32, 48, 64];

/// A square icon as straight (not premultiplied) RGBA.
pub struct Rendered {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// The tray icon at several sizes, drawn in `rgb` (the panel's text colour).
pub fn tray_pixmaps(rgb: [u8; 3]) -> Vec<Rendered> {
    let color = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
    let svg = TRAY_SVG.replace(SVG_TEXT_COLOR, &color);
    let Ok(tree) = usvg::Tree::from_str(&svg, &usvg::Options::default()) else {
        return Vec::new();
    };
    SIZES
        .iter()
        .filter_map(|&size| render(&tree, size))
        .collect()
}

fn render(tree: &usvg::Tree, size: u32) -> Option<Rendered> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia gives premultiplied RGBA.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Some(Rendered { size, rgba })
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_all_sizes() {
        let icons = super::tray_pixmaps([255, 255, 255]);
        assert_eq!(icons.len(), super::SIZES.len());
        // Mostly-transparent corners, opaque body.
        let icon = &icons[3];
        assert!(icon.rgba.chunks(4).any(|px| px[3] == 255));
        assert_eq!(icon.rgba[3], 0);
    }
}
