//! Native menu bar application for `MTPDrive`.

mod application;
mod instance;
mod login_item;
mod service;
mod theme;
mod tray;
mod tray_template;
mod updater;
mod views;

use iced::Size;
use material_ui_rs as material;

pub(crate) const WINDOW_SIZE: Size = Size::new(920.0, 720.0);
const MIN_WINDOW_SIZE: Size = Size::new(620.0, 520.0);

/// Runs the `MTPDrive` menu bar application.
///
/// # Errors
///
/// Returns an error when the graphical runtime cannot be initialized.
pub fn run() -> iced::Result {
    if !instance::acquire() {
        return Ok(());
    }
    let result = material::application(application::boot, application::update, views::view)
        .title("MTPDrive")
        .theme(application::theme)
        .subscription(application::subscription)
        .window(window_settings())
        .exit_on_close_request(false)
        .run();
    instance::release();
    result
}

fn window_settings() -> iced::window::Settings {
    let mut settings = material::window_with_min_size(WINDOW_SIZE, MIN_WINDOW_SIZE);
    settings.visible = false;
    settings
}
