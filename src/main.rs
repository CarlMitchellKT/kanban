mod app;
mod links;
mod model;
mod storage;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Kanban")
            .with_app_id("kanban")
            .with_inner_size([1200.0, 720.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "kanban",
        options,
        Box::new(|_cc| Ok(Box::new(app::KanbanApp::new()))),
    )
}
