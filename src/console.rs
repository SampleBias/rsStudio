//! Console layout and scrolling, independent of the R session.
use eframe::egui::{self, Align, RichText, ScrollArea, TextWrapMode};

pub struct ConsoleView {
    pub run: bool,
    #[cfg(test)]
    pub output: egui::scroll_area::ScrollAreaOutput<()>,
    #[cfg(test)]
    pub input_rect: egui::Rect,
}

pub fn show(
    ui: &mut egui::Ui,
    output: &str,
    input: &mut String,
    ready: bool,
    follow: bool,
) -> ConsoleView {
    let max_input_height = (ui.available_height() - 110.0).max(100.0);
    let default_input_height = (ui.available_height() * 0.25)
        .clamp(125.0, 200.0)
        .min(max_input_height);
    let mut run = false;
    let input_panel = egui::TopBottomPanel::bottom("console_input")
        .resizable(true)
        .default_height(default_input_height)
        .min_height(100.0)
        .max_height(max_input_height)
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("R input");
                if ui
                    .add_enabled(ready, egui::Button::new("Run  Ctrl+Enter"))
                    .clicked()
                {
                    run = true;
                }
            });
            ScrollArea::vertical()
                .id_salt("console_input_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let response = ui.add_enabled(
                        ready,
                        egui::TextEdit::multiline(input)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(4)
                            .hint_text("Type or paste R code here, then press Ctrl+Enter")
                            .desired_width(f32::INFINITY),
                    );
                    if response.has_focus()
                        && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter))
                    {
                        run = true;
                    }
                });
        });
    #[cfg(not(test))]
    let _ = input_panel;

    let mut jump_to_end = follow;
    ui.horizontal(|ui| {
        ui.heading("Console");
        if ui.button("Latest output").clicked() {
            jump_to_end = true;
        }
    });
    ui.weak("Submission console: interactive terminal input is not supported.");
    ui.separator();
    let output_area = ScrollArea::both()
        .id_salt("console_output")
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .animated(false)
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(output).monospace())
                    .wrap_mode(TextWrapMode::Extend)
                    .selectable(true),
            );
            if jump_to_end {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
    #[cfg(not(test))]
    let _ = output_area;
    ConsoleView {
        run,
        #[cfg(test)]
        output: output_area,
        #[cfg(test)]
        input_rect: input_panel.response.rect,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_output_scrolls_to_latest_and_stays_above_input_after_resize() {
        let ctx = egui::Context::default();
        let output = (0..150).map(|n| format!("line {n}\n")).collect::<String>();
        let mut input = String::new();
        for height in [600.0, 320.0] {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(700.0, height),
                )),
                ..Default::default()
            };
            let mut actual = None;
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let view = show(ui, &output, &mut input, true, true);
                    actual = Some((
                        view.output.state.offset.y,
                        view.output.content_size.y,
                        view.output.inner_rect,
                        view.input_rect,
                    ));
                });
            });
            let (offset, content_height, output_rect, input_rect) = actual.unwrap();
            assert!(offset > 0.0, "long output should scroll to its end");
            assert!(
                (offset + output_rect.height() - content_height).abs() < 5.0,
                "latest output must be visible: offset={offset}, viewport={}, content={content_height}",
                output_rect.height()
            );
            assert!(
                output_rect.bottom() <= input_rect.top() + 1.0,
                "output viewport must not overlap the input area"
            );
            assert!(input_rect.height() >= 99.0);
        }
    }
}
