use std::path::PathBuf;

use gpui_kit::*;
use whit::{APP_NAME, AppAssets, RecentFolders, Settings, Workbench, init};

fn main() {
    env_logger::init();
    let initial_folder = std::env::args_os().nth(1).map(PathBuf::from);

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            init(cx);
            // Single-window app: once the window is gone there is nothing left to use.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    Workbench::new(
                        initial_folder,
                        RecentFolders::load(),
                        Settings::load(),
                        window,
                        cx,
                    )
                })
            });
            if let Err(error) = opened {
                log::error!("failed to open window: {error:#}");
                cx.quit();
            }
            cx.activate(true);
        });
}
