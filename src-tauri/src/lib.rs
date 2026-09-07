mod apple;
mod docker;
mod liquid_glass_regions;
mod provider;
mod runtime;

use tauri_plugin_autostart::ManagerExt;

use docker::DockerState;
use provider::{ProviderKind, ProviderState};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    image::Image,
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WebviewUrl, WebviewWindowBuilder,
};

fn send_notification(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let notif = app.notification();

    // Check if notification permission is granted
    let granted = notif
        .permission_state()
        .map(|s| s == tauri_plugin_notification::PermissionState::Granted)
        .unwrap_or(false);

    if granted {
        let _ = notif.builder().title(title).body(body).show();
    }
}

pub struct BrowsingState(pub Arc<AtomicBool>);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(target_os = "macos")]
fn setup_macos_window(window: &tauri::WebviewWindow) {
    use objc2_app_kit::{NSColor, NSWindow};

    let ns_window = window.ns_window().expect("Failed to get NSWindow");
    let ns_window: &NSWindow = unsafe { &*(ns_window as *const _ as *const NSWindow) };

    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
    ns_window.setHasShadow(true);

    // Keep WKWebView as contentView (Tauri already sets _drawsBackground:NO for transparency).
    // CSS backdrop-filter on the .app element blurs the compositor content behind the window.
    if let Some(content_view) = ns_window.contentView() {
        content_view.setWantsLayer(true);
        if let Some(layer) = content_view.layer() {
            layer.setCornerRadius(12.0);
            layer.setMasksToBounds(true);
        }
    }
}

#[cfg(target_os = "macos")]
fn set_macos_webview_window_alpha(window: &tauri::WebviewWindow, alpha: f64) {
    use objc2_app_kit::NSWindow;

    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    let ns_window = ns_window as usize;
    let _ = window.run_on_main_thread(move || {
        let ns_window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };
        ns_window.setAlphaValue(alpha);
    });
}

#[cfg(not(target_os = "macos"))]
fn set_macos_webview_window_alpha(_window: &tauri::WebviewWindow, _alpha: f64) {}

#[cfg(target_os = "macos")]
fn set_macos_window_alpha(window: &tauri::Window, alpha: f64) {
    use objc2_app_kit::NSWindow;

    let Ok(ns_window) = window.ns_window() else {
        return;
    };
    let ns_window = ns_window as usize;
    let _ = window.run_on_main_thread(move || {
        let ns_window: &NSWindow = unsafe { &*(ns_window as *const NSWindow) };
        ns_window.setAlphaValue(alpha);
    });
}

#[cfg(not(target_os = "macos"))]
fn set_macos_window_alpha(_window: &tauri::Window, _alpha: f64) {}

#[derive(Clone, Copy)]
struct LogicalRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn clamped_tray_window_position(
    icon: LogicalRect,
    window_size: (f64, f64),
    work_area: LogicalRect,
) -> (f64, f64) {
    let (window_width, window_height) = window_size;
    let max_x = (work_area.x + work_area.width - window_width).max(work_area.x);
    let max_y = (work_area.y + work_area.height - window_height).max(work_area.y);
    let x = (icon.x + icon.width / 2.0 - window_width / 2.0).clamp(work_area.x, max_x);
    let y = (icon.y + icon.height).clamp(work_area.y, max_y);
    (x, y)
}

#[cfg(test)]
mod tray_position_tests {
    use super::{clamped_tray_window_position, LogicalRect};

    #[test]
    fn centers_below_the_tray_icon() {
        assert_eq!(
            clamped_tray_window_position(
                LogicalRect {
                    x: 1000.0,
                    y: 0.0,
                    width: 24.0,
                    height: 24.0,
                },
                (420.0, 560.0),
                LogicalRect {
                    x: 0.0,
                    y: 24.0,
                    width: 1440.0,
                    height: 876.0,
                },
            ),
            (802.0, 24.0)
        );
    }

    #[test]
    fn keeps_the_window_inside_the_clicked_monitor() {
        assert_eq!(
            clamped_tray_window_position(
                LogicalRect {
                    x: 1430.0,
                    y: 850.0,
                    width: 20.0,
                    height: 24.0,
                },
                (420.0, 560.0),
                LogicalRect {
                    x: 0.0,
                    y: 24.0,
                    width: 1440.0,
                    height: 876.0,
                },
            ),
            (1020.0, 340.0)
        );
        assert_eq!(
            clamped_tray_window_position(
                LogicalRect {
                    x: -100.0,
                    y: 0.0,
                    width: 20.0,
                    height: 24.0,
                },
                (420.0, 560.0),
                LogicalRect {
                    x: -1920.0,
                    y: 24.0,
                    width: 1920.0,
                    height: 1056.0,
                },
            ),
            (-420.0, 24.0)
        );
    }
}

#[cfg(target_os = "macos")]
fn position_window_at_tray_icon(
    window: &tauri::WebviewWindow,
    icon_position: tauri::Position,
    icon_size: tauri::Size,
) {
    // tray-icon reports a physical rect using the clicked screen's scale, but
    // the hidden window still reports the scale of the screen it last occupied.
    // Convert the icon and window to one global logical coordinate space before
    // positioning; mixing those scales sends the popup to another display.
    let Ok(monitors) = window.available_monitors() else {
        return;
    };
    let Ok(Some(primary)) = window.primary_monitor() else {
        return;
    };
    let mouse = objc2_app_kit::NSEvent::mouseLocation();
    let primary_height = primary.size().height as f64 / primary.scale_factor();
    let mouse_x = mouse.x;
    let mouse_y = primary_height - mouse.y;

    let monitor = monitors
        .iter()
        .find(|monitor| {
            let scale = monitor.scale_factor();
            let x = monitor.position().x as f64 / scale;
            let y = monitor.position().y as f64 / scale;
            let width = monitor.size().width as f64 / scale;
            let height = monitor.size().height as f64 / scale;
            mouse_x >= x && mouse_x < x + width && mouse_y >= y && mouse_y < y + height
        })
        .unwrap_or(&primary);

    let target_scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64 / target_scale;
    let monitor_y = monitor.position().y as f64 / target_scale;
    let physical_origin_x = monitor_x * target_scale;
    let physical_origin_y = monitor_y * target_scale;
    let icon_x = match icon_position {
        tauri::Position::Physical(position) => {
            monitor_x + (position.x as f64 - physical_origin_x) / target_scale
        }
        tauri::Position::Logical(position) => position.x,
    };
    let icon_y = match icon_position {
        tauri::Position::Physical(position) => {
            monitor_y + (position.y as f64 - physical_origin_y) / target_scale
        }
        tauri::Position::Logical(position) => position.y,
    };
    let (icon_width, icon_height) = match icon_size {
        tauri::Size::Physical(size) => (
            size.width as f64 / target_scale,
            size.height as f64 / target_scale,
        ),
        tauri::Size::Logical(size) => (size.width, size.height),
    };
    let window_scale = window.scale_factor().unwrap_or(1.0);
    let window_size = window
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(420, 560));
    let window_width = window_size.width as f64 / window_scale;
    let window_height = window_size.height as f64 / window_scale;
    let work_area = monitor.work_area();
    let work_x = work_area.position.x as f64 / target_scale;
    let work_y = work_area.position.y as f64 / target_scale;
    let work_width = work_area.size.width as f64 / target_scale;
    let work_height = work_area.size.height as f64 / target_scale;
    let (x, y) = clamped_tray_window_position(
        LogicalRect {
            x: icon_x,
            y: icon_y,
            width: icon_width,
            height: icon_height,
        },
        (window_width, window_height),
        LogicalRect {
            x: work_x,
            y: work_y,
            width: work_width,
            height: work_height,
        },
    );
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
}

#[cfg(not(target_os = "macos"))]
fn position_window_at_tray_icon(
    window: &tauri::WebviewWindow,
    icon_position: tauri::Position,
    icon_size: tauri::Size,
) {
    let position = icon_position.to_physical(window.scale_factor().unwrap_or(1.0));
    let size = icon_size.to_physical(window.scale_factor().unwrap_or(1.0));
    let window_size = window
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(420, 560));
    let x = position.x + (size.width as i32 - window_size.width as i32) / 2;
    let y = position.y + size.height as i32;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

#[cfg(target_os = "macos")]
fn set_macos_accessory_app() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    if let Some(mtm) = MainThreadMarker::new() {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let last_focus_lost = Arc::new(AtomicU64::new(0));
    let last_focus_lost_for_tray = last_focus_lost.clone();
    let browsing = Arc::new(AtomicBool::new(false));
    let browsing_for_event = browsing.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_liquid_glass::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(DockerState {
            client: Arc::new(std::sync::Mutex::new(None)),
        })
        .manage(BrowsingState(browsing))
        .manage(liquid_glass_regions::RegionGlassState::default())
        .manage(RuntimeState {
            starting: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
        })
        .manage(ProviderState::new(ProviderKind::default()))
        .invoke_handler(tauri::generate_handler![
            docker::list_containers,
            docker::list_images,
            docker::list_volumes,
            docker::list_networks,
            docker::start_container,
            docker::stop_container,
            docker::restart_container,
            docker::start_container_group,
            docker::stop_container_group,
            docker::get_container_logs,
            docker::docker_ping,
            docker::remove_container,
            docker::remove_image,
            docker::remove_volume,
            docker::remove_network,
            docker::pull_image,
            docker::create_container,
            docker::compose_up,
            docker::inspect_compose_conflicts,
            docker::stop_conflicting_compose_projects,
            docker::get_container_logs_since,
            docker::get_container_env,
            docker::get_container_mounts,
            docker::open_in_finder,
            docker::detect_terminal,
            docker::open_terminal,
            docker::list_container_files,
            docker::read_container_file,
            docker::save_from_container,
            docker::import_to_container,
            runtime_status,
            runtime_overview,
            switch_provider,
            runtime_start,
            runtime_stop,
            get_provider,
            get_vm_config,
            apply_vm_config,
            get_autostart,
            set_autostart,
            get_app_version,
            check_for_updates,
            open_log_window,
            open_file_explorer_window,
            get_home_dir,
            pick_file_for_import,
            pick_yaml_file,
            liquid_glass_regions::set_liquid_glass_regions,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            set_macos_accessory_app();

            // Load the persisted provider selection into the in-memory cache.
            let provider = provider::load_provider(app.handle());
            app.state::<ProviderState>().set(provider);
            if provider != ProviderKind::Apple {
                if let Ok(mut guard) = app.state::<DockerState>().client.lock() {
                    *guard = runtime::connect_provider(provider);
                }
            }

            let icon = app
                .path()
                .resource_dir()
                .ok()
                .and_then(|dir| Image::from_path(dir.join("icons/tray-icon.png")).ok())
                .or_else(|| Image::from_path("icons/tray-icon.png").ok())
                .expect("Failed to load tray icon");

            let autostart_manager = app.autolaunch();
            let is_autostart = autostart_manager.is_enabled().unwrap_or(false);

            let autostart_item = CheckMenuItemBuilder::with_id("autostart", "Start at Login")
                .checked(is_autostart)
                .build(app)
                .expect("Failed to build autostart menu item");
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Containbar")
                .build(app)
                .expect("Failed to build quit menu item");
            let tray_menu = MenuBuilder::new(app)
                .item(&autostart_item)
                .separator()
                .item(&quit_item)
                .build()
                .expect("Failed to build tray menu");

            let _tray = TrayIconBuilder::with_id("docker-tray")
                .icon(icon)
                .icon_as_template(true)
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .tooltip("Containbar")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "autostart" => {
                        let manager = app.autolaunch();
                        let enabled = manager.is_enabled().unwrap_or(false);
                        if enabled {
                            let _ = manager.disable();
                        } else {
                            let _ = manager.enable();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(move |tray, event| {
                    if let TrayIconEvent::Click {
                        rect,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        let window = match app.get_webview_window("main") {
                            Some(w) => w,
                            None => WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                                .title("Containbar")
                                .inner_size(420.0, 560.0)
                                .decorations(false)
                                .skip_taskbar(true)
                                .always_on_top(true)
                                .transparent(true)
                                .visible(false)
                                .build()
                                .expect("Failed to create window"),
                        };

                        // Record that a tray click happened
                        last_focus_lost_for_tray.store(now_ms(), Ordering::SeqCst);

                        if window.is_visible().unwrap_or(false) {
                            let _ = window.hide();
                            return;
                        }

                        {
                            position_window_at_tray_icon(&window, rect.position, rect.size);
                            #[cfg(target_os = "macos")]
                            {
                                use objc2::MainThreadMarker;
                                use objc2_app_kit::NSApplication;
                                if let Some(mtm) = MainThreadMarker::new() {
                                    let ns_app = NSApplication::sharedApplication(mtm);
                                    #[allow(deprecated)]
                                    ns_app.activateIgnoringOtherApps(true);
                                }
                            }
                            set_macos_webview_window_alpha(&window, 1.0);
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                setup_macos_window(&window);
                if provider::setup_complete(app.handle()) {
                    let _ = window.hide();
                } else {
                    let _ = window.center();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }

            // Existing users keep automatic startup for app-managed runtimes.
            // A first run stays idle so the setup wizard can explain and ask
            // before installing anything.
            let provider = app.state::<ProviderState>().get();
            let status =
                runtime::detect_runtime(&app.path().resource_dir().unwrap_or_default(), provider);
            let setup_complete = provider::setup_complete(app.handle());
            let needs_start = setup_complete && provider != ProviderKind::Docker && !status.running;

            if needs_start {
                let resource_dir = app.path().resource_dir().unwrap_or_default();
                let docker_client = app.state::<DockerState>().client.clone();
                let starting = app.state::<RuntimeState>().starting.clone();
                let error = app.state::<RuntimeState>().error.clone();
                let app_handle = app.handle().clone();

                if !starting.load(Ordering::SeqCst) {
                    starting.store(true, Ordering::SeqCst);
                    if let Some(tray) = app_handle.tray_by_id("docker-tray") {
                        let _ = tray.set_tooltip(Some("Containbar — Starting runtime..."));
                    }
                    std::thread::spawn(move || {
                        let success = match prepare_provider(&app_handle, &resource_dir, provider) {
                            Ok(client) => {
                                if let Ok(mut guard) = docker_client.lock() {
                                    *guard = client;
                                }
                                true
                            }
                            Err(message) => {
                                if let Ok(mut guard) = error.lock() {
                                    *guard = Some(message);
                                }
                                false
                            }
                        };
                        starting.store(false, Ordering::SeqCst);
                        if let Some(tray) = app_handle.tray_by_id("docker-tray") {
                            let _ = tray.set_tooltip(Some(if success {
                                "Containbar"
                            } else {
                                "Containbar — Runtime failed"
                            }));
                        }
                        if success {
                            send_notification(&app_handle, "Containbar", "Runtime is ready");
                        } else {
                            send_notification(&app_handle, "Containbar", "Runtime failed to start");
                        }
                    });
                }
            } else if setup_complete && provider == ProviderKind::Apple && status.running {
                // The Apple backend may survive an app restart, but it has no
                // daemon-level restart policy. Reconcile persisted Compose
                // intent without pretending that the runtime itself is down.
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    match docker::restore_apple_compose_projects(&app_handle) {
                        Ok(restored) if restored > 0 => send_notification(
                            &app_handle,
                            "Apple Compose restored",
                            &format!("Restarted {restored} service(s)"),
                        ),
                        Ok(_) => {}
                        Err(error) => {
                            eprintln!("{error}");
                            send_notification(
                                &app_handle,
                                "Apple Compose restore incomplete",
                                &error,
                            );
                        }
                    }
                });
            }

            Ok(())
        })
        .on_window_event(move |window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::Focused(focused) = event {
                    if *focused {
                        set_macos_window_alpha(window, 1.0);
                        return;
                    }

                    if browsing_for_event.load(Ordering::SeqCst) {
                        return;
                    }

                    // Remove the window from the current frame before AppKit
                    // redraws native glass with its darker inactive material.
                    set_macos_window_alpha(window, 0.0);
                    let w = window.clone();
                    let ts = last_focus_lost.clone();
                    let br = browsing_for_event.clone();
                    // Delay hide to let tray click events arrive first
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(150));
                        // Skip hide if file picker is open
                        if br.load(Ordering::SeqCst) {
                            set_macos_window_alpha(&w, 1.0);
                            return;
                        }
                        if w.is_focused().unwrap_or(false) {
                            set_macos_window_alpha(&w, 1.0);
                            return;
                        }
                        let clicked_at = ts.load(Ordering::SeqCst);
                        let elapsed = now_ms() - clicked_at;
                        // If a tray click happened within 150ms, skip hide
                        if elapsed < 200 {
                            return;
                        }
                        let _ = w.hide();
                    });
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
async fn open_log_window(
    app: tauri::AppHandle,
    container_id: String,
    container_name: String,
) -> Result<(), String> {
    let label = format!("log-{}", container_id);

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return Ok(());
    }

    let url = format!("index.html#/logs/{}/{}", container_id, container_name);

    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .title(format!("Logs: {}", container_name))
        .inner_size(960.0, 600.0)
        .min_inner_size(600.0, 400.0)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn open_file_explorer_window(
    app: tauri::AppHandle,
    container_id: String,
    container_name: String,
) -> Result<(), String> {
    let label = format!("files-{}", container_id);

    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return Ok(());
    }

    let url = format!("index.html#/files/{}/{}", container_id, container_name);

    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .title(format!("Files: {}", container_name))
        .inner_size(800.0, 600.0)
        .min_inner_size(500.0, 400.0)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn get_home_dir() -> Result<String, String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "Cannot determine home directory".to_string())
}

#[tauri::command]
async fn pick_file_for_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, BrowsingState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let flag = state.0.clone();
    flag.store(true, Ordering::SeqCst);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_file(move |file| {
        let path = file
            .and_then(|file| file.into_path().ok())
            .map(|path| path.to_string_lossy().to_string());
        let _ = sender.send(path);
    });
    let result = receiver
        .await
        .map_err(|_| "File picker closed unexpectedly".to_string());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        flag.store(false, Ordering::SeqCst);
    });
    result
}

#[tauri::command]
async fn pick_yaml_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, BrowsingState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let flag = state.0.clone();
    flag.store(true, Ordering::SeqCst);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Docker Compose", &["yaml", "yml"])
        .pick_file(move |file| {
            let path = file
                .and_then(|file| file.into_path().ok())
                .map(|path| path.to_string_lossy().to_string());
            let _ = sender.send(path);
        });
    let result = receiver
        .await
        .map_err(|_| "File picker closed unexpectedly".to_string());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        flag.store(false, Ordering::SeqCst);
    });
    result
}

// --- Runtime management ---

use std::sync::Mutex;

struct RuntimeState {
    starting: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

#[derive(Serialize)]
struct RuntimeOverview {
    setup_complete: bool,
    selected: ProviderKind,
    recommended: ProviderKind,
    providers: Vec<runtime::ProviderStatus>,
}

fn build_runtime_overview(app: &tauri::AppHandle, selected: ProviderKind) -> RuntimeOverview {
    let resource_dir = app.path().resource_dir().unwrap_or_default();
    let setup_complete = provider::setup_complete(app);
    RuntimeOverview {
        setup_complete,
        selected,
        recommended: if setup_complete {
            selected
        } else {
            runtime::suggested_provider(&resource_dir)
        },
        providers: [
            ProviderKind::Docker,
            ProviderKind::Colima,
            ProviderKind::Apple,
        ]
        .into_iter()
        .map(|provider| runtime::provider_status(&resource_dir, provider))
        .collect(),
    }
}

fn prepare_provider(
    app: &tauri::AppHandle,
    resource_dir: &std::path::Path,
    provider: ProviderKind,
) -> Result<Option<bollard::Docker>, String> {
    match provider {
        ProviderKind::Docker => runtime::connect_provider(provider)
            .map(Some)
            .ok_or_else(|| {
                "Docker Desktop or OrbStack is not running. Start it, then try again.".to_string()
            }),
        ProviderKind::Colima => {
            if runtime::connect_provider(provider).is_none() {
                runtime::start_builtin(resource_dir)?;
            }
            runtime::connect_provider(provider)
                .map(Some)
                .ok_or_else(|| "Colima started, but its Docker socket did not respond.".to_string())
        }
        ProviderKind::Apple => {
            runtime::ensure_apple_container()?;
            apple::system_start()?;
            apple::list_containers()?;
            if let Err(error) = docker::restore_apple_compose_projects(app) {
                eprintln!("{error}");
                send_notification(app, "Apple Compose restore incomplete", &error);
            }
            Ok(None)
        }
    }
}

#[tauri::command]
fn runtime_overview(
    app: tauri::AppHandle,
    provider_state: tauri::State<'_, ProviderState>,
) -> RuntimeOverview {
    build_runtime_overview(&app, provider_state.get())
}

#[tauri::command]
async fn switch_provider(
    app: tauri::AppHandle,
    provider: ProviderKind,
    provider_state: tauri::State<'_, ProviderState>,
    docker: tauri::State<'_, DockerState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<RuntimeOverview, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    if runtime_state.starting.swap(true, Ordering::SeqCst) {
        return Err("Another runtime operation is already in progress.".to_string());
    }

    if let Ok(mut guard) = runtime_state.error.lock() {
        *guard = None;
    }
    let resource_for_task = resource_dir.clone();
    let app_for_task = app.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        prepare_provider(&app_for_task, &resource_for_task, provider)
    })
    .await;

    runtime_state.starting.store(false, Ordering::SeqCst);
    let result = joined.map_err(|error| error.to_string())?;
    match result {
        Ok(client) => {
            provider::store_provider(&app, provider)?;
            provider_state.set(provider);
            if let Ok(mut guard) = docker.client.lock() {
                *guard = client;
            }
            Ok(build_runtime_overview(&app, provider))
        }
        Err(error) => {
            if let Ok(mut guard) = runtime_state.error.lock() {
                *guard = Some(error.clone());
            }
            Err(error)
        }
    }
}

#[tauri::command]
fn runtime_status(
    app: tauri::AppHandle,
    provider_state: tauri::State<'_, ProviderState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<runtime::RuntimeStatus, String> {
    let provider = provider_state.get();
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let mut status = runtime::detect_runtime(&resource_dir, provider);
    status.provider = provider;

    let is_starting = runtime_state.starting.load(Ordering::SeqCst);

    // Override with starting state
    if is_starting {
        status.running = false;
        status.message = "Starting runtime...".to_string();
        // Keep the detected kind but reflect that something is starting; the
        // frontend gates the "Start" button on `running: false` + message.
        if provider == ProviderKind::Apple {
            status.kind = runtime::RuntimeKind::Apple;
        } else {
            status.kind = runtime::RuntimeKind::Builtin;
        }
    }

    // Check for errors — only consume when not starting (avoid brief flash)
    if !is_starting {
        if let Ok(mut guard) = runtime_state.error.lock() {
            if let Some(err) = guard.take() {
                status.kind = runtime::RuntimeKind::None;
                status.running = false;
                status.message = err;
            }
        }
    }

    Ok(status)
}

#[tauri::command]
fn runtime_start(
    app: tauri::AppHandle,
    provider_state: tauri::State<'_, ProviderState>,
    docker: tauri::State<'_, DockerState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<(), String> {
    if runtime_state.starting.load(Ordering::SeqCst) {
        return Ok(()); // Already starting
    }

    let provider = provider_state.get();

    if provider != ProviderKind::Apple {
        if let Some(client) = runtime::connect_provider(provider).and_then(|client| {
            tauri::async_runtime::block_on(client.ping())
                .ok()
                .map(|_| client)
        }) {
            if let Ok(mut guard) = docker.client.lock() {
                *guard = Some(client);
            }
            return Ok(());
        }
    }

    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let docker_client = docker.client.clone();
    let starting = runtime_state.starting.clone();
    let error = runtime_state.error.clone();

    // Clear previous error
    if let Ok(mut guard) = error.lock() {
        *guard = None;
    }
    starting.store(true, Ordering::SeqCst);

    // Update tray tooltip
    if let Some(tray) = app.tray_by_id("docker-tray") {
        let _ = tray.set_tooltip(Some("Containbar — Starting runtime..."));
    }

    let app_handle = app.clone();

    // Run in background thread — returns immediately
    std::thread::spawn(move || {
        let success = match prepare_provider(&app_handle, &resource_dir, provider) {
            Ok(client) => {
                if let Ok(mut guard) = docker_client.lock() {
                    *guard = client;
                }
                true
            }
            Err(message) => {
                if let Ok(mut guard) = error.lock() {
                    *guard = Some(message);
                }
                false
            }
        };
        starting.store(false, Ordering::SeqCst);

        // Update tray tooltip
        if let Some(tray) = app_handle.tray_by_id("docker-tray") {
            let _ = tray.set_tooltip(Some(if success {
                "Containbar"
            } else {
                "Containbar — Runtime failed"
            }));
        }

        // Send macOS notification
        if success {
            send_notification(&app_handle, "Containbar", "Runtime is ready");
        } else {
            send_notification(&app_handle, "Containbar", "Runtime failed to start");
        }
    });

    Ok(())
}

#[tauri::command]
fn runtime_stop(
    app: tauri::AppHandle,
    provider_state: tauri::State<'_, ProviderState>,
) -> Result<String, String> {
    let provider = provider_state.get();
    match provider {
        ProviderKind::Apple => {
            // Apple Container has no clean "stop backend"; best-effort we leave
            // it running (it is lightweight). Report success.
            Ok("Apple Container backend left running".to_string())
        }
        ProviderKind::Docker => Ok("External Docker runtime left running".to_string()),
        ProviderKind::Colima => {
            let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
            runtime::stop_builtin(&resource_dir)
        }
    }
}

// --- Provider selection ---

#[tauri::command]
fn get_provider(provider_state: tauri::State<'_, ProviderState>) -> ProviderKind {
    provider_state.get()
}

#[tauri::command]
fn get_vm_config() -> runtime::VmConfig {
    runtime::read_vm_config()
}

#[tauri::command]
fn apply_vm_config(
    app: tauri::AppHandle,
    config: runtime::VmConfig,
    docker: tauri::State<'_, DockerState>,
    runtime_state: tauri::State<'_, RuntimeState>,
) -> Result<(), String> {
    if runtime_state.starting.load(Ordering::SeqCst) {
        return Err("Runtime is already starting".to_string());
    }

    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let docker_client = docker.client.clone();
    let starting = runtime_state.starting.clone();
    let error = runtime_state.error.clone();

    if let Ok(mut guard) = error.lock() {
        *guard = None;
    }
    starting.store(true, Ordering::SeqCst);

    if let Some(tray) = app.tray_by_id("docker-tray") {
        let _ = tray.set_tooltip(Some("Containbar — Restarting runtime..."));
    }

    let app_handle = app.clone();

    std::thread::spawn(move || {
        // Stop first
        let _ = runtime::stop_builtin(&resource_dir);
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Start with new config
        let success = match runtime::start_builtin_with_config(&resource_dir, &config) {
            Ok(_) => {
                std::thread::sleep(std::time::Duration::from_secs(2));
                match runtime::connect_provider(ProviderKind::Colima) {
                    Some(client) => {
                        if let Ok(mut guard) = docker_client.lock() {
                            *guard = Some(client);
                        }
                        true
                    }
                    None => {
                        if let Ok(mut guard) = error.lock() {
                            *guard = Some(
                                "Runtime started but Docker connection failed. Try again."
                                    .to_string(),
                            );
                        }
                        false
                    }
                }
            }
            Err(e) => {
                if let Ok(mut guard) = error.lock() {
                    *guard = Some(e);
                }
                false
            }
        };
        starting.store(false, Ordering::SeqCst);

        if let Some(tray) = app_handle.tray_by_id("docker-tray") {
            let _ = tray.set_tooltip(Some(if success {
                "Containbar"
            } else {
                "Containbar — Runtime failed"
            }));
        }

        if success {
            send_notification(
                &app_handle,
                "Containbar",
                "Runtime restarted with new settings",
            );
        } else {
            send_notification(&app_handle, "Containbar", "Runtime failed to restart");
        }
    });

    Ok(())
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    Ok(app.autolaunch().is_enabled().unwrap_or(false))
}

#[tauri::command]
fn get_app_version(app: tauri::AppHandle) -> String {
    app.package_info().version.to_string()
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    current_version: String,
    latest_version: String,
    has_update: bool,
    release_url: String,
}

fn version_is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv > cv {
            return true;
        }
        if lv < cv {
            return false;
        }
    }
    false
}

#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();

    let client = reqwest::Client::builder()
        .user_agent("containbar-updater")
        .build()
        .map_err(|e| e.to_string())?;

    let resp: serde_json::Value = client
        .get("https://api.github.com/repos/yurseria/containbar/releases/latest")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let tag = resp["tag_name"].as_str().ok_or("No release found")?;
    let latest = tag.trim_start_matches('v').to_string();
    let release_url = resp["html_url"]
        .as_str()
        .unwrap_or("https://github.com/yurseria/containbar/releases")
        .to_string();

    Ok(UpdateInfo {
        has_update: version_is_newer(&latest, &current),
        current_version: current,
        latest_version: latest,
        release_url,
    })
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}
