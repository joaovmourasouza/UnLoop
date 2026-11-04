mod app;
mod checkin_page;
mod detail;
mod entry;
mod focus;
mod habit_form;
mod home;
mod settings;
mod widgets;

use adw::prelude::*;

pub const APP_ID: &str = "io.github.joaovmourasouza.UnLoop";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> gtk::glib::ExitCode {
    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(app::activate);
    application.run()
}
