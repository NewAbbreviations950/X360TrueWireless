//! The tray icon, rendered from our SVG and sent as pixels. Tray hosts look icons up by name
//! inconsistently (and cache misses), so pixels are the reliable option.

use resvg::{tiny_skia, usvg};

const TRAY_SVG: &str = include_str!("../../../packaging/icons/x360truewireless-tray-symbolic.svg");
/// The colour the SVG's stylesheet uses for `currentColor`; replaced with the panel's text colour.
const SVG_TEXT_COLOR: &str = "#232629";
const SIZES: [u32; 5] = [16, 22, 32, 48, 64];

/// The tray icon at several sizes, drawn in `rgb` (the panel's text colour).
pub fn tray_pixmaps(rgb: [u8; 3]) -> Vec<ksni::Icon> {
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

fn render(tree: &usvg::Tree, size: u32) -> Option<ksni::Icon> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia gives premultiplied RGBA; StatusNotifierItem wants straight ARGB, big-endian.
    let data = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.alpha(), c.red(), c.green(), c.blue()]
        })
        .collect();
    Some(ksni::Icon {
        width: size as i32,
        height: size as i32,
        data,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_all_sizes() {
        let icons = super::tray_pixmaps([255, 255, 255]);
        assert_eq!(icons.len(), super::SIZES.len());
        // Mostly-transparent corners, opaque body.
        let icon = &icons[3];
        assert!(icon.data.chunks(4).any(|px| px[0] == 255));
        assert_eq!(icon.data[0], 0);
    }
}
