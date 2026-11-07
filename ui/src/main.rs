//! crabsim's UI: a Bevy application with egui panels (ADR 0006). So far it only opens a window.
//! UI stage 1 is described in docs/agents/handoff.md, "UI design".
//!
//! cargo run -p crabsim-ui --profile ui

use bevy::{log::LogPlugin, prelude::*, winit::WinitSettings};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "crabsim".into(), ..default() }),
                    ..default()
                })
                // The engine logs every event at info level, which Bevy's logger would print, and
                // the graphics driver's Vulkan loader warns about duplicate layers on every start.
                .set(LogPlugin { filter: "warn,crabsim=warn,wgpu_hal=error".into(), ..default() }),
        )
        .add_plugins(EguiPlugin::default())
        // Draw only when something changes, so an idle window costs nothing.
        .insert_resource(WinitSettings::desktop_app())
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        })
        .add_systems(EguiPrimaryContextPass, top_bar)
        .run();
}

fn top_bar(mut contexts: EguiContexts) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    egui::Panel::top("top").show(&mut root, |ui| {
        ui.label("crabsim");
    });
    Ok(())
}
