//! About window with the license and third-party notices, shared by the editor and player.

pub const LICENSE: &str = include_str!("../LICENSE-MIT");
pub const THIRD_PARTY: &str = include_str!("../THIRD_PARTY_LICENSES.txt");

pub fn window(ctx: &egui::Context, open: &mut bool, app: &str) {
    egui::Window::new(format!("About {app}")).open(open).default_size([620.0, 520.0]).resizable(true).show(ctx, |ui| {
        ui.heading(format!("{app} {}", env!("CARGO_PKG_VERSION")));
        ui.label("Copyright © 2026 Jasper Gulka. Open source under the MIT or Apache-2.0 license.");
        ui.hyperlink_to("github.com/atlis-svg/flint", "https://github.com/atlis-svg/flint");
        ui.label("Movies, games and projectors you make with Flint are yours to share.");
        ui.label(egui::RichText::new("AI disclosure: most of Flint's code was written by Claude (Anthropic) via Claude Code, directed and reviewed by Jasper Gulka.").small().weak());
        ui.separator();
        egui::CollapsingHeader::new("License (MIT; Apache-2.0 also applies, see LICENSE-APACHE)").show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("lic").max_height(200.0).show(ui, |ui| {
                ui.monospace(LICENSE);
            });
        });
        ui.strong("Open-source components");
        ui.label(egui::RichText::new("Flint is built with these libraries and fonts, used under their own licenses:").small());
        // the notices are ~700 KB, so only lay out the visible lines
        let lines: Vec<&str> = THIRD_PARTY.lines().collect();
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical().id_salt("3p").auto_shrink(false).show_rows(ui, row_h, lines.len(), |ui, range| {
            for l in &lines[range] {
                ui.label(egui::RichText::new(*l).monospace().size(11.0));
            }
        });
    });
}
