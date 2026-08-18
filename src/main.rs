#![warn(missing_debug_implementations)]
#![cfg_attr(
    not(any(debug_assertions, feature = "debug-mode")),
    windows_subsystem = "windows"
)]
#[macro_use]
pub mod util;
pub mod app;
pub mod config;
pub mod error;
pub mod events;
pub mod font;
pub mod localization;

use glutin::dpi::{LogicalSize, PhysicalPosition};
use glutin::window::Icon;
use glutin_window::GlutinWindow;
use opengl_graphics::{GlGraphics, OpenGL};
use piston::window::WindowSettings;

use crate::app::App;
use crate::events::launch;

use std::env;
use std::io;
use std::path::PathBuf;

const WINDOW_WIDTH_MIN: u32 = 384;
const WINDOW_HEIGHT_MIN: u32 = 256;

pub const PRODUCT_NAME: &str = "HOI4 Map Editor";
pub const PRODUCT_SUBTITLE: &str = "Province and State Editing Toolkit";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APPNAME: &str = concat!("HOI4 Map Editor v", env!("CARGO_PKG_VERSION"));
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/app-icon-256.png");

fn main() {
    install_handler();

    let root = root_dir().expect("unable to find root dir");
    env::set_current_dir(root).expect("unable to set root dir");
    let global_config = crate::config::GlobalConfig::load()
        .map(|loaded| loaded.value)
        .unwrap_or_default();
    crate::localization::set_language(&global_config.language);

    let opengl = OpenGL::V3_2;
    let screen = [
        global_config.window.width.max(WINDOW_WIDTH_MIN),
        global_config.window.height.max(WINDOW_HEIGHT_MIN),
    ];
    let mut window: GlutinWindow = WindowSettings::new(APPNAME, screen)
        .graphics_api(opengl)
        .resizable(true)
        .vsync(true)
        .build()
        .expect("unable to initialize window");
    let icon = application_icon().expect("assets/app-icon-256.png must decode as a window icon");
    window.ctx.window().set_window_icon(Some(icon));
    if let (Some(x), Some(y)) = (global_config.window.x, global_config.window.y) {
        let visible = window.ctx.window().available_monitors().any(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            x >= position.x - screen[0] as i32
                && x < position.x + size.width as i32
                && y >= position.y - screen[1] as i32
                && y < position.y + size.height as i32
        });
        if visible {
            window
                .ctx
                .window()
                .set_outer_position(PhysicalPosition::new(x, y));
        }
    }
    window
        .ctx
        .window()
        .set_maximized(global_config.window.maximized);
    let screen_min = LogicalSize::new(WINDOW_WIDTH_MIN, WINDOW_HEIGHT_MIN);
    window.ctx.window().set_min_inner_size(Some(screen_min));
    let mut gl = GlGraphics::new(opengl);
    launch::<App>(&mut window, &mut gl);
}

fn application_icon() -> Result<Icon, String> {
    let image = image::load_from_memory(APP_ICON_PNG)
        .map_err(|err| format!("decode assets/app-icon-256.png: {err}"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|err| format!("create window icon from assets/app-icon-256.png: {err}"))
}

fn root_dir() -> io::Result<PathBuf> {
    if let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") {
        return Ok(PathBuf::from(manifest_dir));
    };

    let mut current_exe = dunce::canonicalize(env::current_exe()?)?;

    if current_exe.pop() {
        return Ok(current_exe);
    };

    Err(io::Error::new(
        io::ErrorKind::Other,
        "failed to find an application root",
    ))
}

use std::io::prelude::*;

fn write_application_info(mut out: impl Write) -> Result<(), std::io::Error> {
    writeln!(out, "Application: {}", APPNAME)?;
    writeln!(out, "Version: v{}", APP_VERSION)?;
    writeln!(out, "Operating System: {}", env::consts::OS)?;
    writeln!(out, "Architecture: {}", env::consts::ARCH)?;
    writeln!(
        out,
        "Debug Assertions Enabled: {:?}",
        cfg!(debug_assertions)
    )?;
    writeln!(
        out,
        "Debug Mode Feature Enabled: {:?}",
        cfg!(feature = "debug-mode")
    )?;
    writeln!(out)?;

    Ok(())
}

pub(crate) fn diagnostic_summary() -> String {
    let mut output = Vec::new();
    write_application_info(&mut output).expect("writing application info to memory cannot fail");
    String::from_utf8(output).expect("application information is valid UTF-8")
}

pub(crate) fn log_directory() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("HOI4MapEditor")
        .join("logs")
}

fn install_handler() {
    use chrono::Local;
    use color_backtrace::{BacktracePrinter, Verbosity};
    use fs_err::File;
    use termcolor::NoColor;

    use std::panic::{PanicHookInfo, set_hook};
    use std::sync::Mutex;

    let printer = BacktracePrinter::new()
        .verbosity(Verbosity::Full)
        .lib_verbosity(Verbosity::Full)
        .clear_frame_filters();
    let out = Mutex::new(color_backtrace::default_output_stream());
    set_hook(Box::new(move |pi: &PanicHookInfo| {
        // if either of these are enabled, the console is enabled (on windows)
        if cfg!(any(debug_assertions, feature = "debug-mode")) {
            let mut out_lock = out.lock().unwrap();
            if let Err(err) = printer.print_panic_info(pi, &mut *out_lock) {
                eprintln!("Error while printing panic: {err:?}");
            };
        };

        // only write panic info to file if not on dev profile
        if cfg!(not(debug_assertions)) {
            let now = Local::now().format("%Y%m%d_%H%M%S");
            let log_dir = log_directory();
            let log_path = log_dir.join(format!("crash_{}.log", now));
            let create_result =
                std::fs::create_dir_all(&log_dir).and_then(|_| File::create(&log_path));
            match create_result {
                Ok(out_file) => {
                    if let Err(err) = write_application_info(&out_file) {
                        eprintln!("Error while printing application info: {err:?}");
                    };

                    if let Err(err) = printer.print_panic_info(pi, &mut NoColor::new(&out_file)) {
                        eprintln!("Error while printing panic: {err:?}");
                    };
                }
                Err(e) => eprintln!(
                    "Error creating crash log at {}: {:?}",
                    log_path.display(),
                    e
                ),
            };
        };
    }));
}

#[cfg(test)]
mod branding_tests {
    use super::{
        APP_ICON_PNG, APP_VERSION, APPNAME, PRODUCT_NAME, diagnostic_summary, log_directory,
    };

    #[test]
    fn embedded_application_icon_is_valid_png() {
        let icon = image::load_from_memory(APP_ICON_PNG)
            .expect("assets/app-icon-256.png should be a valid embedded PNG")
            .into_rgba8();
        assert_eq!(icon.dimensions(), (256, 256));
        assert_eq!(icon.as_raw().len(), 256 * 256 * 4);
    }

    #[test]
    fn public_window_title_uses_map_editor_branding_without_version_bump() {
        assert_eq!(APPNAME, "HOI4 Map Editor v0.1.0-preview.2");
        assert_eq!(PRODUCT_NAME, "HOI4 Map Editor");
        assert_eq!(APP_VERSION, env!("CARGO_PKG_VERSION"));
        assert!(diagnostic_summary().contains("Operating System:"));
        assert!(log_directory().ends_with("HOI4MapEditor\\logs"));
    }
}
