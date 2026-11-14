use eframe::egui::{self, Color32, Visuals};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

pub struct Settings {
    pub dark: bool,
    pub font_size: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: true,
            font_size: 14.0,
        }
    }
}

fn path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))?;
    Some(config.join("rsstudio").join("settings"))
}

impl Settings {
    pub fn load() -> Self {
        let mut settings = Self::default();
        if let Some(contents) = path().and_then(|p| fs::read_to_string(p).ok()) {
            for line in contents.lines() {
                if let Some(value) = line.strip_prefix("theme=") {
                    settings.dark = value != "light";
                }
                if let Some(value) = line.strip_prefix("font_size=")
                    && let Ok(size) = value.parse::<f32>()
                    && size.is_finite()
                    && (10.0..=24.0).contains(&size)
                {
                    settings.font_size = size;
                }
            }
        }
        settings
    }
    pub fn save(&self) -> io::Result<()> {
        let path =
            path().ok_or_else(|| io::Error::other("Could not locate a configuration directory"))?;
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("Invalid configuration path"))?;
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(".settings-{}.tmp", std::process::id()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            writeln!(
                file,
                "theme={}\nfont_size={}",
                if self.dark { "dark" } else { "light" },
                self.font_size
            )?;
            file.sync_all()?;
            fs::rename(&temp, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        if self.dark {
            visuals.panel_fill = Color32::from_rgb(27, 31, 39);
            visuals.window_fill = Color32::from_rgb(35, 40, 49);
            visuals.extreme_bg_color = Color32::from_rgb(21, 24, 31);
            visuals.selection.bg_fill = Color32::from_rgb(57, 77, 108);
        } else {
            visuals.panel_fill = Color32::from_rgb(247, 246, 242);
            visuals.window_fill = Color32::from_rgb(255, 254, 250);
            visuals.extreme_bg_color = Color32::from_rgb(255, 255, 255);
            visuals.selection.bg_fill = Color32::from_rgb(176, 205, 237);
        }
        ctx.set_visuals(visuals);
    }
}
