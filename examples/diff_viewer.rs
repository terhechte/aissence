//! A tiny windowed demo of the syntax-highlighted [`aissence::DiffView`].
//!
//! ```text
//! # show a built-in sample patch
//! cargo run --example diff_viewer
//!
//! # show a real commit's diff from any repo
//! cargo run --example diff_viewer -- /path/to/repo            # HEAD
//! cargo run --example diff_viewer -- /path/to/repo <commit>   # a specific commit
//! ```

use std::path::PathBuf;

use aissence::DiffView;
use aissence::git;

const SAMPLE_PATCH: &str = r#"diff --git a/src/lib.rs b/src/lib.rs
index 1234567..89abcde 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,6 +1,9 @@
 //! A small example module.

-pub fn greet(name: &str) -> String {
-    format!("Hi {}", name)
+/// Greet someone by name.
+pub fn greet(name: &str) -> String {
+    // Friendlier wording + punctuation.
+    format!("Hello, {name}!")
 }

 pub const MAX_RETRIES: usize = 3;
"#;

struct DiffApp {
    title: String,
    patch: String,
}

impl DiffApp {
    fn from_args() -> Self {
        let mut args = std::env::args().skip(1);
        let Some(folder) = args.next().map(PathBuf::from) else {
            return Self {
                title: "sample patch".to_string(),
                patch: SAMPLE_PATCH.to_string(),
            };
        };
        let commit = args.next().unwrap_or_else(|| "HEAD".to_string());

        match git::commit_patch(&folder, &commit) {
            Ok(patch) => Self {
                title: format!("{} @ {commit}", folder.display()),
                patch,
            },
            Err(err) => Self {
                title: format!("error: {err} — showing sample instead"),
                patch: SAMPLE_PATCH.to_string(),
            },
        }
    }
}

impl eframe::App for DiffApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());
        egui::TopBottomPanel::top("title").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("aissence — diff viewer");
            ui.label(egui::RichText::new(&self.title).weak());
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            DiffView::from_patch(&self.patch).show(ui);
        });
    }
}

fn main() -> eframe::Result {
    let app = DiffApp::from_args();
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "aissence diff viewer",
        options,
        Box::new(|_cc| Ok(Box::new(app))),
    )
}
