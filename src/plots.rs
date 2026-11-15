//! In-memory plot history and the output-pane presentation.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};
use std::io::{Cursor, Write};
use std::path::Path;

const HISTORY_LIMIT: usize = 30;

struct Plot {
    page: u64,
    png: Vec<u8>,
    texture: TextureHandle,
}

#[derive(Default)]
pub struct Plots {
    history: Vec<Plot>,
    current: usize,
    zoom_open: bool,
}

impl Plots {
    pub fn clear(&mut self) {
        self.history.clear();
        self.current = 0;
        self.zoom_open = false;
    }

    pub fn receive(&mut self, ctx: &egui::Context, page: u64, png: Vec<u8>) -> Result<(), String> {
        let reader = image::ImageReader::with_format(Cursor::new(&png), image::ImageFormat::Png);
        let (width, height) = reader
            .into_dimensions()
            .map_err(|e| format!("Cannot read plot: {e}"))?;
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err("Plot dimensions exceed the display limit.".into());
        }
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .map_err(|e| format!("Cannot decode plot: {e}"))?
            .into_rgba8();
        let color =
            ColorImage::from_rgba_unmultiplied([width as usize, height as usize], image.as_raw());
        let texture = ctx.load_texture(format!("plot-{page}"), color, TextureOptions::LINEAR);
        let plot = Plot { page, png, texture };
        if let Some(index) = self.history.iter().position(|p| p.page == page) {
            self.history[index] = plot;
        } else {
            self.history.push(plot);
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
            self.current = self.history.len() - 1;
        }
        Ok(())
    }

    pub fn export_snapshot(&self, path: &Path) -> Result<Vec<u8>, String> {
        let plot = self.history.get(self.current).ok_or("No plot to export.")?;
        if !path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("png"))
        {
            return Err("Use a .png filename for plot export.".into());
        }
        Ok(plot.png.clone())
    }
    pub fn write_export(path: &Path, png: &[u8]) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(png)
    }
    #[cfg(test)]
    fn export(&self, path: &Path) -> Result<(), String> {
        let png = self.export_snapshot(path)?;
        Self::write_export(path, &png).map_err(|e| e.to_string())
    }

    /// Draw the pane, returning true when the user requests a PNG export.
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut export = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.current > 0 && !self.history.is_empty(),
                    egui::Button::new("‹"),
                )
                .on_hover_text("Previous plot")
                .clicked()
            {
                self.current -= 1;
            }
            if ui
                .add_enabled(
                    self.current + 1 < self.history.len(),
                    egui::Button::new("›"),
                )
                .on_hover_text("Next plot")
                .clicked()
            {
                self.current += 1;
            }
            if self.history.is_empty() {
                ui.weak("0 / 0");
            } else {
                ui.label(format!("{} / {}", self.current + 1, self.history.len()));
            }
            if ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("Zoom"))
                .clicked()
            {
                self.zoom_open = true;
            }
            if ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("Export PNG…"))
                .clicked()
            {
                export = true;
            }
            if ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("Remove"))
                .clicked()
            {
                self.history.remove(self.current);
                self.current = self.current.min(self.history.len().saturating_sub(1));
            }
            if ui
                .add_enabled(!self.history.is_empty(), egui::Button::new("Clear"))
                .clicked()
            {
                self.clear();
            }
        });
        ui.separator();
        if let Some(plot) = self.history.get(self.current) {
            show_plot(ui, &plot.texture, ui.available_size());
        } else {
            ui.weak("Run R plotting code to see a plot here.");
        }
        export
    }

    pub fn show_zoom(&mut self, ctx: &egui::Context) {
        if let Some(plot) = self.history.get(self.current) {
            egui::Window::new("Plot zoom")
                .open(&mut self.zoom_open)
                .default_size([900.0, 700.0])
                .resizable(true)
                .show(ctx, |ui| show_plot(ui, &plot.texture, ui.available_size()));
        }
    }
}

fn show_plot(ui: &mut egui::Ui, texture: &TextureHandle, available: Vec2) {
    let original = texture.size_vec2();
    let available = available.max(Vec2::splat(1.0));
    let scale = (available.x / original.x).min(available.y / original.y);
    let size = original * scale;
    ui.centered_and_justified(|ui| {
        ui.add(egui::Image::new((texture.id(), size)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(value: u8) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([value, 0, 0, 255]));
        let mut data = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut data, image::ImageFormat::Png)
            .unwrap();
        data.into_inner()
    }
    #[test]
    fn history_replaces_layers_and_retains_separate_pages() {
        let ctx = egui::Context::default();
        let mut plots = Plots::default();
        plots.receive(&ctx, 1, png(1)).unwrap();
        plots.receive(&ctx, 1, png(2)).unwrap();
        assert_eq!(plots.history.len(), 1);
        assert_eq!(plots.history[0].png, png(2));
        plots.receive(&ctx, 2, png(3)).unwrap();
        assert_eq!(plots.history.len(), 2);
        assert_eq!(plots.current, 1);
        assert!(plots.receive(&ctx, 3, b"not png".to_vec()).is_err());
        plots.clear();
        assert!(plots.history.is_empty());
    }
    #[test]
    fn export_never_overwrites_existing_files() {
        let ctx = egui::Context::default();
        let mut plots = Plots::default();
        plots.receive(&ctx, 1, png(1)).unwrap();
        let path =
            std::env::temp_dir().join(format!("rsstudio-plot-test-{}.png", std::process::id()));
        plots.export(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), png(1));
        assert!(plots.export(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn plot_is_painted_inside_the_output_pane() {
        let ctx = egui::Context::default();
        let mut plots = Plots::default();
        plots.receive(&ctx, 1, png(1)).unwrap();
        let texture_id = plots.history[0].texture.id();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 350.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    plots.show(ui);
                });
            },
        );
        let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
        let plot_mesh = meshes
            .iter()
            .find_map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture_id => Some(mesh),
                _ => None,
            })
            .expect("plot texture should be painted");
        assert!(
            plot_mesh
                .vertices
                .iter()
                .all(|v| v.pos.x >= 0.0 && v.pos.x <= 400.0 && v.pos.y >= 0.0 && v.pos.y <= 350.0)
        );
    }
}
