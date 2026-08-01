mod app;
mod build;
mod cli;
mod config;
mod error;
mod layout;
mod logging;
mod modding;
mod process;
mod runtime;
mod sandbox;
mod source;
mod toolchain;
mod util;

fn main() {
    if let Err(error) = app::run() {
        if log::log_enabled!(log::Level::Error) {
            log::error!("{error}");
        } else {
            eprintln!("error: {error}");
        }
        std::process::exit(1);
    }
}
