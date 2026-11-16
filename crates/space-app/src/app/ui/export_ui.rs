//! Export helpers (CSV, TSV, JSON, SVG, PNG screenshot, basket list) and macOS Share sheet.

use std::path::Path;

use eframe::egui;
use space_core::export;

use crate::app::SpaceApp;
use crate::largest_files::InsightTab;

impl SpaceApp {
    fn save_text_export(&mut self, default_name: &str, contents: String) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, contents) {
            Ok(()) => {
                self.status = crate::tr!("Saved {}", path.display());
                self.last_exported_file = Some(path);
            }
            Err(e) => {
                self.status = crate::tr!("Export failed: {}", e);
            }
        }
    }

    pub(super) fn export_children_csv(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let csv = export::children_csv(&tree, self.focus, self.size_mode, 0);
        self.save_text_export("folder-contents.csv", csv);
    }

    pub(super) fn export_largest_csv(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let csv = export::largest_csv(
            &tree,
            self.focus,
            self.size_mode,
            self.settings.largest_limit,
        );
        self.save_text_export("largest-files.csv", csv);
    }

    pub(super) fn export_insights_csv(&mut self, ids: &[space_core::NodeId]) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        if self.insight_tab == InsightTab::Types {
            let mut csv = String::from("extension,file_count,total_bytes\n");
            for (ext, bytes, count) in &self.largest.data().types {
                let label = if ext.is_empty() { "(none)" } else { ext };
                csv.push_str(&format!("{label},{count},{bytes}\n"));
            }
            self.save_text_export("file-types.csv", csv);
        } else {
            let csv = export::rows_csv(&tree, ids, self.size_mode);
            self.save_text_export("insights.csv", csv);
        }
    }

    pub(super) fn export_tsv(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let mut nodes = vec![self.focus];
        nodes.extend_from_slice(tree.children(self.focus));
        let tsv = export::nodes_tsv(&tree, &nodes, self.size_mode);
        self.save_text_export("folder-contents.tsv", tsv);
    }

    pub(super) fn export_json_shallow(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let json = export::json_shallow(&tree, self.focus, self.size_mode, 200);
        self.save_text_export("folder-summary.json", json);
    }

    pub(super) fn export_json_full(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        match export::json_full(&tree) {
            Ok(json) => self.save_text_export("space-tree.json", json),
            Err(e) => self.status = crate::tr!("Export failed: {}", e),
        }
    }

    pub(super) fn export_sunburst_svg(&mut self) {
        let Some(tree) = self.tree.clone() else {
            return;
        };
        let svg = export::sunburst_svg(
            &tree,
            self.focus,
            self.size_mode,
            self.settings.sunburst_rings,
            1024,
        );
        self.save_text_export("space-rings.svg", svg);
    }

    pub(super) fn export_basket_list(&mut self) {
        let items = self.basket_items();
        if items.is_empty() {
            return;
        }
        let mut out = String::from("bytes,path\n");
        for item in items {
            let s = item.path.to_string_lossy();
            let escaped = if s.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.into_owned()
            };
            out.push_str(&format!("{},{}\n", item.bytes, escaped));
        }
        self.save_text_export("trash-basket.csv", out);
    }

    pub(super) fn request_png_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("space-map.png")
            .save_file()
        else {
            return;
        };
        self.pending_screenshot = Some(path);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }

    pub(in crate::app) fn handle_screenshot_events(&mut self, ctx: &egui::Context) {
        if self.pending_screenshot.is_none() {
            return;
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|ev| match ev {
                egui::Event::Screenshot { image, .. } => Some(std::sync::Arc::clone(image)),
                _ => None,
            })
        });
        let Some(image) = shot else {
            return;
        };
        let Some(path) = self.pending_screenshot.take() else {
            return;
        };
        match write_png(&path, &image) {
            Ok(()) => {
                self.status = crate::tr!("Saved {}", path.display());
                self.last_exported_file = Some(path);
            }
            Err(e) => {
                self.status = crate::tr!("Export failed: {}", e);
            }
        }
    }

    pub(super) fn share_report_macos(&mut self) {
        let target = self
            .last_exported_file
            .clone()
            .filter(|p| p.exists())
            .or_else(|| {
                let tree = self.tree.as_ref()?;
                let path = std::env::temp_dir().join("space-scanner-report.svg");
                let svg = export::sunburst_svg(
                    tree,
                    self.focus,
                    self.size_mode,
                    self.settings.sunburst_rings,
                    1024,
                );
                std::fs::write(&path, svg).ok()?;
                Some(path)
            });
        let Some(path) = target else {
            self.status =
                crate::i18n::t("Nothing to share yet — export or scan a folder first.").into();
            return;
        };
        if !show_macos_share_sheet(&path) {
            self.status = crate::tr!("Share sheet unavailable — saved at {}", path.display());
        }
    }
}

fn write_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let w = std::io::BufWriter::new(file);
    let mut enc = png::Encoder::new(w, image.size[0] as u32, image.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(image.as_raw())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn show_macos_share_sheet(path: &Path) -> bool {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSSharingServicePicker};
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(window) = app.keyWindow().or_else(|| app.mainWindow()) else {
        return false;
    };
    let Some(view) = window.contentView() else {
        return false;
    };
    let path_str = NSString::from_str(&path.to_string_lossy());
    let url = NSURL::fileURLWithPath(&path_str);
    let obj: Retained<AnyObject> = Retained::into_super(Retained::into_super(url));
    let items = NSArray::from_retained_slice(&[obj]);
    let picker =
        unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items) };
    let bounds = view.bounds();
    let anchor = NSRect::new(
        NSPoint::new(
            bounds.origin.x + bounds.size.width * 0.5 - 12.0,
            bounds.origin.y + bounds.size.height - 40.0,
        ),
        NSSize::new(24.0, 24.0),
    );
    picker.showRelativeToRect_ofView_preferredEdge(anchor, &view, NSRectEdge::MinY);
    true
}

#[cfg(not(target_os = "macos"))]
fn show_macos_share_sheet(_path: &Path) -> bool {
    false
}
