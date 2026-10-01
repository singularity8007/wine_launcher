//! The window. It draws what `Backend::view()` says and forwards button
//! clicks to `Backend`. It never touches downloads, threads or the game, so
//! swapping this file for a prettier one later changes nothing else.

use crate::backend::Backend;
use eframe::egui;
use std::time::Duration;

pub fn run(backend: Backend) -> eframe::Result {
    eframe::run_native(
        "wine_launcher",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(App { backend }) as Box<dyn eframe::App>)),
    )
}

struct App {
    backend: Backend,
}

impl eframe::App for App {
    // FIXED: eframe 0.36 uses `ui` method providing `&mut egui::Ui` instead of `Context`
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Since we have a `Ui` instead of a `Context`, we use `ui.ctx()` to trigger a repaint
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        let view = self.backend.view();

        // FIXED: Removed egui::CentralPanel since `ui` is already the root viewport UI container!
        ui.heading("wine_launcher");
        ui.label("Zenless Zone Zero");
        ui.add_space(12.0);

        // Greyed out while a job runs, so two jobs can't start at once.
        ui.add_enabled_ui(!view.busy, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Launch").clicked() {
                    self.backend.launch();
                }
                if ui.button("Install / update ZZZ").clicked() {
                    self.backend.install_game();
                }
                if ui.button("Check for updates").clicked() {
                    self.backend.check_updates();
                }
            });
        });
        ui.add_space(12.0);

        if view.busy {
            match view.fraction {
                Some(f) => ui.add(egui::ProgressBar::new(f).show_percentage()),
                None => ui.add(egui::Spinner::new()),


                
            };
            ui.label(&view.label);
        }

        if let Some(message) = &view.message {
            let color = if message.is_error {
                egui::Color32::LIGHT_RED
            } else {
                egui::Color32::LIGHT_GREEN
            };
            ui.colored_label(color, &message.text);
        }
    }
}