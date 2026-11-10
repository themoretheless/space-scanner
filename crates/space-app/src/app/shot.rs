//! Map exports: a PNG screenshot of the map area and the rings as SVG.

use eframe::egui;

use super::{export_stem, ui, SpaceApp};

impl SpaceApp {
    /// Ask the window for a screenshot; it arrives as `Event::Screenshot`.
    pub(super) fn request_map_png(&mut self, ctx: &egui::Context) {
        if self.map_rect.is_none() || self.exporting {
            return;
        }
        self.shot_pending = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }

    /// Crop an arrived screenshot to the map, encode it and save it.
    pub(super) fn take_screenshot(&mut self, ctx: &egui::Context) {
        if !self.shot_pending {
            return;
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else {
            ctx.request_repaint();
            return;
        };
        self.shot_pending = false;
        let Some(rect) = self.map_rect else { return };
        let ppp = ctx.pixels_per_point();
        let crop = image.region(&rect, Some(ppp));
        match encode_png(&crop) {
            Ok(bytes) => {
                let name = self
                    .tree
                    .as_ref()
                    .map(|t| export_stem(&t.node(self.focus).name))
                    .unwrap_or_else(|| "map".into());
                self.export_bytes(&format!("{name}-map.png"), bytes);
            }
            Err(e) => self.notify(
                ui::ToastLevel::Error,
                crate::tr!("Could not encode the image: {}", e),
            ),
        }
    }

    /// Save the cached ring geometry as SVG.
    pub(super) fn export_sunburst_svg(&mut self) {
        let (Some(tree), Some(geom)) = (self.tree.clone(), self.geom.as_ref()) else {
            return;
        };
        let Some(svg) = crate::views::sunburst_svg(geom, &tree, self.size_mode, crate::theme::BG())
        else {
            return;
        };
        let name = export_stem(&tree.node(self.focus).name);
        self.export_text(&format!("{name}-rings.svg"), svg);
    }
}

/// RGBA PNG of an egui image.
pub(crate) fn encode_png(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let [w, h] = image.size;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in &image.pixels {
        rgba.extend_from_slice(&px.to_srgba_unmultiplied());
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_has_the_signature_and_size() {
        let img = egui::ColorImage::new([3, 2], egui::Color32::RED);
        let bytes = encode_png(&img).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR width and height, big-endian.
        assert_eq!(&bytes[16..24], &[0, 0, 0, 3, 0, 0, 0, 2]);
    }
}
