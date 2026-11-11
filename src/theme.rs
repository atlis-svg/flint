//! Editor themes: named palettes, an accent colour and per-user persistence.
//!
//! A theme covers the editor chrome only — panels, windows, text and the
//! pasteboard around the stage. Artwork colours live in the document.

use crate::model::Rgba;
use egui::{Color32, CornerRadius, Stroke, style::WidgetVisuals};
use serde::{Deserialize, Serialize};

/// The chrome colours of the editor.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Palette {
    /// Side panels and the menu bar.
    pub panel: Rgba,
    /// Windows and popups.
    pub window: Rgba,
    /// The darkest wells: text edits, scroll bars.
    pub bg: Rgba,
    /// Main text.
    pub fg: Rgba,
    /// Secondary text.
    pub muted: Rgba,
    /// The pasteboard around the stage.
    pub stage: Rgba,
}

/// A theme: a palette, a light/dark mode and an optional accent override.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Theme {
    /// The preset this theme came from.
    pub name: String,
    pub dark: bool,
    pub palette: Palette,
    /// Overrides the preset's accent colour. `None` = use the preset's.
    #[serde(default)]
    pub accent: Option<Rgba>,
}

/// A named theme with a light and a dark variant.
pub struct Preset {
    pub name: &'static str,
    /// The mode the theme starts in when it is picked.
    pub default_dark: bool,
    pub accent: Rgba,
    pub dark: Palette,
    pub light: Palette,
}

const fn pal(panel: [u8; 4], window: [u8; 4], bg: [u8; 4], fg: [u8; 4], muted: [u8; 4], stage: [u8; 4]) -> Palette {
    Palette { panel, window, bg, fg, muted, stage }
}

/// The themes shipped with Flint.
pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Midnight",
        default_dark: true,
        accent: [76, 141, 255, 255],
        dark: pal([35, 38, 46, 255], [27, 30, 38, 255], [20, 22, 28, 255], [230, 232, 240, 255], [139, 145, 162, 255], [58, 63, 76, 255]),
        light: pal([240, 242, 246, 255], [250, 250, 252, 255], [228, 231, 238, 255], [26, 28, 34, 255], [104, 110, 124, 255], [214, 217, 224, 255]),
    },
    Preset {
        name: "Graphite",
        default_dark: true,
        accent: [240, 160, 42, 255],
        dark: pal([43, 43, 46, 255], [35, 35, 38, 255], [26, 26, 28, 255], [232, 232, 234, 255], [144, 144, 150, 255], [60, 60, 64, 255]),
        light: pal([238, 238, 240, 255], [248, 248, 250, 255], [222, 222, 226, 255], [24, 24, 26, 255], [110, 110, 116, 255], [206, 206, 210, 255]),
    },
    Preset {
        name: "Solarized Dark",
        default_dark: true,
        accent: [181, 137, 0, 255],
        dark: pal([7, 54, 66, 255], [6, 48, 59, 255], [0, 43, 54, 255], [147, 161, 161, 255], [88, 110, 117, 255], [7, 54, 66, 255]),
        light: pal([238, 232, 213, 255], [245, 239, 220, 255], [253, 246, 227, 255], [7, 54, 66, 255], [131, 148, 150, 255], [238, 232, 213, 255]),
    },
    Preset {
        name: "Solarized Light",
        default_dark: false,
        accent: [203, 75, 22, 255],
        dark: pal([7, 54, 66, 255], [6, 48, 59, 255], [0, 43, 54, 255], [147, 161, 161, 255], [88, 110, 117, 255], [7, 54, 66, 255]),
        light: pal([238, 232, 213, 255], [245, 239, 220, 255], [253, 246, 227, 255], [7, 54, 66, 255], [131, 148, 150, 255], [238, 232, 213, 255]),
    },
    Preset {
        name: "Paper",
        default_dark: false,
        accent: [47, 111, 237, 255],
        dark: pal([30, 30, 33, 255], [24, 24, 27, 255], [16, 16, 18, 255], [240, 240, 244, 255], [130, 130, 140, 255], [44, 44, 48, 255]),
        light: pal([242, 242, 245, 255], [250, 250, 252, 255], [255, 255, 255, 255], [27, 27, 31, 255], [106, 106, 117, 255], [220, 220, 226, 255]),
    },
    Preset {
        name: "Nord",
        default_dark: true,
        accent: [136, 192, 208, 255],
        dark: pal([59, 66, 82, 255], [46, 52, 64, 255], [36, 41, 51, 255], [236, 239, 244, 255], [123, 136, 161, 255], [59, 66, 82, 255]),
        light: pal([229, 233, 240, 255], [236, 239, 244, 255], [216, 222, 233, 255], [46, 52, 64, 255], [104, 112, 130, 255], [207, 216, 230, 255]),
    },
    Preset {
        name: "Dracula",
        default_dark: true,
        accent: [189, 147, 249, 255],
        dark: pal([52, 55, 70, 255], [40, 42, 54, 255], [30, 31, 41, 255], [248, 248, 242, 255], [98, 114, 164, 255], [52, 55, 70, 255]),
        light: pal([245, 243, 250, 255], [252, 251, 255, 255], [236, 234, 244, 255], [40, 42, 54, 255], [120, 122, 148, 255], [228, 226, 238, 255]),
    },
    Preset {
        name: "Rose Pine",
        default_dark: true,
        accent: [235, 188, 186, 255],
        dark: pal([31, 29, 46, 255], [25, 23, 36, 255], [18, 16, 28, 255], [224, 222, 244, 255], [110, 106, 134, 255], [31, 29, 46, 255]),
        light: pal([250, 244, 237, 255], [255, 250, 243, 255], [242, 233, 225, 255], [87, 82, 121, 255], [152, 147, 165, 255], [235, 225, 215, 255]),
    },
    Preset {
        name: "Amber",
        default_dark: true,
        accent: [255, 128, 0, 255],
        dark: pal([36, 26, 0, 255], [26, 19, 0, 255], [18, 13, 0, 255], [255, 176, 0, 255], [176, 128, 0, 255], [46, 34, 0, 255]),
        light: pal([255, 246, 214, 255], [255, 250, 230, 255], [240, 228, 180, 255], [60, 40, 0, 255], [150, 110, 20, 255], [226, 210, 160, 255]),
    },
    Preset {
        name: "High Contrast",
        default_dark: true,
        accent: [255, 212, 0, 255],
        dark: pal([16, 16, 16, 255], [24, 24, 24, 255], [0, 0, 0, 255], [255, 255, 255, 255], [176, 176, 176, 255], [32, 32, 32, 255]),
        light: pal([255, 255, 255, 255], [240, 240, 240, 255], [255, 255, 255, 255], [0, 0, 0, 255], [70, 70, 70, 255], [200, 200, 200, 255]),
    },
];

impl Default for Theme {
    fn default() -> Self {
        Theme::preset("Midnight").unwrap_or_else(|| Theme {
            name: "Midnight".into(),
            dark: true,
            palette: PRESETS[0].dark,
            accent: None,
        })
    }
}

impl Theme {
    /// The theme with the given preset name.
    pub fn preset(name: &str) -> Option<Theme> {
        let p = PRESETS.iter().find(|p| p.name == name)?;
        Some(Theme { name: p.name.to_string(), dark: p.default_dark, palette: if p.default_dark { p.dark } else { p.light }, accent: None })
    }

    /// Switches to a preset, in the mode it starts in.
    pub fn apply_preset(&mut self, name: &str) -> bool {
        let Some(p) = PRESETS.iter().find(|p| p.name == name) else { return false };
        self.name = p.name.to_string();
        self.dark = p.default_dark;
        self.palette = if self.dark { p.dark } else { p.light };
        self.accent = None;
        true
    }

    /// Switches between the light and dark variant of the current preset.
    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        if let Some(p) = PRESETS.iter().find(|p| p.name == self.name) {
            self.palette = if dark { p.dark } else { p.light };
        }
    }

    /// The accent colour: the user's override, or the preset's.
    pub fn accent(&self) -> Rgba {
        self.accent.unwrap_or_else(|| PRESETS.iter().find(|p| p.name == self.name).map(|p| p.accent).unwrap_or([76, 141, 255, 255]))
    }

    /// The accent colour as egui sees it.
    pub fn accent_color(&self) -> Color32 {
        c(self.accent())
    }

    /// Builds egui's visuals from the palette.
    pub fn visuals(&self) -> egui::Visuals {
        let p = self.palette;
        let accent = self.accent();
        let mut v = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.dark_mode = self.dark;
        v.panel_fill = c(p.panel);
        v.window_fill = c(p.window);
        v.extreme_bg_color = c(p.bg);
        v.faint_bg_color = c(mix(p.panel, p.fg, 0.04));
        v.code_bg_color = c(mix(p.bg, p.fg, 0.06));
        v.override_text_color = None;
        v.weak_text_color = Some(c(p.muted));
        v.weak_text_alpha = 1.0;
        v.hyperlink_color = c(accent);
        v.selection.bg_fill = c(accent);
        // selected labels (tool buttons, toggles) draw their text in this
        // colour, so it has to read on the accent itself
        v.selection.stroke = Stroke::new(1.0, contrast_text(accent));
        v.window_stroke = Stroke::new(1.0, c(mix(p.window, p.fg, 0.12)));
        v.text_edit_bg_color = None;

        let fg = c(p.fg);
        let border = Stroke::new(1.0, c(mix(p.panel, p.fg, if self.dark { 0.16 } else { 0.14 })));
        let well = c(mix(p.bg, p.fg, if self.dark { 0.08 } else { 0.05 }));
        let button = c(mix(p.panel, p.fg, if self.dark { 0.10 } else { 0.07 }));
        let hover = c(mix(p.panel, accent, if self.dark { 0.28 } else { 0.18 }));
        let press = c(mix(p.panel, accent, if self.dark { 0.48 } else { 0.34 }));
        let r = CornerRadius::same(6);
        v.widgets.noninteractive = WidgetVisuals {
            bg_fill: c(p.window),
            weak_bg_fill: c(p.window),
            bg_stroke: border,
            fg_stroke: Stroke::new(1.0, fg),
            corner_radius: CornerRadius::same(4),
            expansion: 0.0,
        };
        v.widgets.inactive = WidgetVisuals { bg_fill: well, weak_bg_fill: button, bg_stroke: border, fg_stroke: Stroke::new(1.0, fg), corner_radius: r, expansion: 0.0 };
        v.widgets.hovered = WidgetVisuals { bg_fill: well, weak_bg_fill: hover, bg_stroke: border, fg_stroke: Stroke::new(1.0, fg), corner_radius: r, expansion: 0.0 };
        v.widgets.active = WidgetVisuals { bg_fill: well, weak_bg_fill: press, bg_stroke: border, fg_stroke: Stroke::new(1.0, fg), corner_radius: r, expansion: 0.0 };
        v.widgets.open = v.widgets.hovered;
        v
    }

    /// The theme file, next to the Hub login.
    pub fn path() -> Option<std::path::PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .or_else(|| if cfg!(windows) { std::env::var_os("APPDATA") } else { None })
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
        Some(base.join("flint").join("theme.json"))
    }

    /// The saved theme, or the default one.
    pub fn load() -> Theme {
        Theme::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Saves the theme for the next start.
    pub fn save(&self) -> Result<(), String> {
        let Some(p) = Theme::path() else { return Ok(()) };
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

/// Blends two colours.
pub fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// Black or white, whichever reads on the given colour.
pub fn contrast_text(col: Rgba) -> Color32 {
    let lum = 0.299 * col[0] as f32 + 0.587 * col[1] as f32 + 0.114 * col[2] as f32;
    if lum > 150.0 { Color32::BLACK } else { Color32::WHITE }
}

/// A document colour as an egui colour.
pub fn c(r: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(r[0], r[1], r[2], r[3])
}
