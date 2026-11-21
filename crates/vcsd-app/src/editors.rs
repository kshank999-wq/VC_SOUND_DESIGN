//! Plugins' own editor windows.
//!
//! Each open editor gets a native window without a webview; the plugin's view
//! (VST3 `IPlugView`, AU Cocoa view) is attached to it. Everything here runs on
//! the main thread (GUI toolkits and plugin views require it), so editors live
//! in a thread-local map. The editor keeps its plugin instance alive until the
//! window closes, even if the slot is removed meanwhile.
//!
//! Linux: plugins embed X11 windows and join the GTK main loop through VST3's
//! `IRunLoop`, implemented with GLib sources.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WindowEvent};
use vcsd_engine::vcsd_plugins::{Editor, EditorHost, Instance, ParentWindow};
use vcsd_engine::Engine;

struct Open {
    // field order = drop order: detach the view, then release the instance
    editor: Box<dyn Editor>,
    slot: String,
    inst: Arc<dyn Instance>,
    host: Rc<Host>,
    window: tauri::Window,
}

thread_local! {
    static OPEN: RefCell<HashMap<String, Open>> = RefCell::new(HashMap::new());
}

/// Number of open editors (readable off the main thread).
static OPEN_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn count_open() {
    OPEN_COUNT.store(OPEN.with(|o| o.borrow().len()), std::sync::atomic::Ordering::Relaxed);
}

/// While editors are open, show automated values in their controls (~20×/s, main thread).
fn start_display_sync(app: &AppHandle) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    let app = app.clone();
    STARTED.call_once(move || {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(50));
            if OPEN_COUNT.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                let _ = app.run_on_main_thread(|| OPEN.with(|o| o.borrow().values().for_each(|x| x.inst.sync_display())));
            }
        });
    });
}

fn label(slot: &str) -> String {
    format!("fx-{}", slot.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_', "_"))
}

/// Editor sizes are points on macOS, physical pixels elsewhere.
fn to_size(width: u32, height: u32) -> tauri::Size {
    if cfg!(target_os = "macos") {
        tauri::LogicalSize::new(width as f64, height as f64).into()
    } else {
        tauri::PhysicalSize::new(width, height).into()
    }
}

struct Host {
    window: tauri::Window,
    /// The size we last gave the window, and when (window systems resize
    /// asynchronously: earlier size events can still arrive after it).
    wanted: Cell<(u32, u32)>,
    set_at: Cell<Instant>,
    /// Editor can't be resized by the user: the window is pinned to its size.
    fixed: Cell<bool>,
    #[cfg(target_os = "linux")]
    run_loop: Rc<glib_loop::GlibLoop>,
}

impl Host {
    fn apply(&self, width: u32, height: u32) {
        self.wanted.set((width, height));
        self.set_at.set(Instant::now());
        let s = to_size(width, height);
        if self.fixed.get() {
            let _ = self.window.set_min_size(Some(s));
            let _ = self.window.set_max_size(Some(s));
        }
        let _ = self.window.set_size(s);
    }
}

impl EditorHost for Host {
    fn resize(&self, width: u32, height: u32) -> bool {
        self.apply(width, height);
        true
    }
    fn scale(&self) -> f64 {
        if cfg!(target_os = "macos") {
            1.0 // AppKit scales views itself
        } else {
            self.window.scale_factor().unwrap_or(1.0)
        }
    }
    #[cfg(target_os = "linux")]
    fn run_loop(&self) -> Option<Rc<dyn vcsd_engine::vcsd_plugins::RunLoop>> {
        Some(self.run_loop.clone())
    }
}

#[cfg(target_os = "linux")]
mod glib_loop {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use vcsd_engine::vcsd_plugins::RunLoop;

    /// VST3 run loop on the GTK main loop (plugin fds and timers).
    #[derive(Default)]
    pub struct GlibLoop {
        next: Cell<u64>,
        sources: RefCell<HashMap<u64, glib::SourceId>>,
    }

    impl GlibLoop {
        fn add(&self, s: glib::SourceId) -> u64 {
            self.next.set(self.next.get() + 1);
            self.sources.borrow_mut().insert(self.next.get(), s);
            self.next.get()
        }
    }

    impl RunLoop for GlibLoop {
        fn watch_fd(&self, fd: i32, f: Box<dyn Fn()>) -> u64 {
            self.add(glib::source::unix_fd_add_local(fd, glib::IOCondition::IN, move |_, _| {
                f();
                glib::ControlFlow::Continue
            }))
        }
        fn every(&self, ms: u32, f: Box<dyn Fn()>) -> u64 {
            self.add(glib::timeout_add_local(std::time::Duration::from_millis(ms as u64), move || {
                f();
                glib::ControlFlow::Continue
            }))
        }
        fn cancel(&self, id: u64) {
            if let Some(s) = self.sources.borrow_mut().remove(&id) {
                s.remove();
            }
        }
    }

    impl Drop for GlibLoop {
        fn drop(&mut self) {
            for (_, s) in self.sources.borrow_mut().drain() {
                s.remove();
            }
        }
    }
}

// X11 ids are C `unsigned long`: u64 on Linux, u32 elsewhere
#[allow(clippy::unnecessary_cast)]
fn parent_of(w: &tauri::Window) -> Result<ParentWindow, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let h = w.window_handle().map_err(|e| e.to_string())?.as_raw();
    match h {
        RawWindowHandle::Win32(h) => Ok(ParentWindow::Win32(h.hwnd.get() as *mut std::ffi::c_void)),
        RawWindowHandle::AppKit(h) => Ok(ParentWindow::AppKit(h.ns_view.as_ptr())),
        RawWindowHandle::Xlib(h) => Ok(ParentWindow::X11(h.window as u64)),
        RawWindowHandle::Xcb(h) => Ok(ParentWindow::X11(h.window.get() as u64)),
        RawWindowHandle::Wayland(_) => Err("plugin windows need X11: start the app with GDK_BACKEND=x11".into()),
        _ => Err("plugin windows aren't supported on this window system".into()),
    }
}

/// Open (or bring to the front) a plugin's own editor. Main thread.
fn open_on_main(app: &AppHandle, engine: &Engine, slot: &str, title: &str) -> Result<(), String> {
    let lbl = label(slot);
    let existing = OPEN.with(|o| o.borrow().get(&lbl).map(|x| x.window.clone()));
    if let Some(w) = existing {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let inst = engine.rack.instance(slot).ok_or("the plugin isn't loaded")?;
    let window = tauri::Window::builder(app, &lbl).title(title).inner_size(400.0, 300.0).build().map_err(|e| e.to_string())?;
    let host = Rc::new(Host {
        window: window.clone(),
        wanted: Cell::new((0, 0)),
        set_at: Cell::new(Instant::now()),
        fixed: Cell::new(false),
        #[cfg(target_os = "linux")]
        run_loop: Rc::new(glib_loop::GlibLoop::default()),
    });
    let opened = parent_of(&window).and_then(|p| inst.open_editor(p, host.clone()));
    let editor = match opened {
        Ok(e) => e,
        Err(e) => {
            let _ = window.destroy();
            return Err(e);
        }
    };
    let (w, h) = editor.size();
    host.fixed.set(!editor.resizable());
    let _ = window.set_maximizable(editor.resizable());
    host.apply(w, h);
    let key = lbl.clone();
    window.on_window_event(move |ev| match ev {
        WindowEvent::Resized(size) => {
            let k = key.clone();
            let size = *size;
            OPEN.with(|o| {
                let o = o.borrow();
                let Some(x) = o.get(&k) else { return };
                let (w, h) = if cfg!(target_os = "macos") {
                    let s = x.window.scale_factor().unwrap_or(1.0);
                    ((size.width as f64 / s) as u32, (size.height as f64 / s) as u32)
                } else {
                    (size.width, size.height)
                };
                let wanted = x.host.wanted.get();
                if (w, h) == wanted {
                    return;
                }
                // a late event from before our own resize, or a fixed-size editor: hold our size
                if x.host.fixed.get() || x.host.set_at.get().elapsed() < Duration::from_millis(500) {
                    x.host.apply(wanted.0, wanted.1);
                    return;
                }
                // the user resized it: the plugin may adjust (size constraints)
                let got = x.editor.set_size(w, h);
                x.host.wanted.set(got);
                if got != (w, h) {
                    x.host.apply(got.0, got.1);
                }
            });
        }
        // detach the view before the window goes away
        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => {
            OPEN.with(|o| o.borrow_mut().remove(&key));
            count_open();
        }
        _ => {}
    });
    OPEN.with(|o| o.borrow_mut().insert(lbl, Open { editor, slot: slot.to_string(), inst, host, window }));
    count_open();
    start_display_sync(app);
    Ok(())
}

fn close_on_main(slot: &str) {
    let removed = OPEN.with(|o| o.borrow_mut().remove(&label(slot)));
    count_open();
    if let Some(x) = removed {
        let window = x.window.clone();
        drop(x); // detach first
        let _ = window.destroy();
    }
}

/// Run `f` on the main thread and wait for its result.
async fn on_main<T: Send + 'static>(app: &AppHandle, f: impl FnOnce(&AppHandle) -> T + Send + 'static) -> Result<T, String> {
    let (tx, rx) = tokio_oneshot();
    let a = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&a));
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || rx.recv().map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}

fn tokio_oneshot<T>() -> (std::sync::mpsc::Sender<T>, std::sync::mpsc::Receiver<T>) {
    std::sync::mpsc::channel()
}

/// Open the plugin's own editor window. Errors (no editor, unsupported window
/// system) let the UI fall back to its parameter panel.
#[tauri::command]
pub async fn plugin_editor_open(app: AppHandle, slot: String, title: String) -> Result<(), String> {
    let engine = app.state::<Arc<Engine>>().inner().clone();
    on_main(&app, move |a| open_on_main(a, &engine, &slot, &title)).await?
}

#[tauri::command]
pub async fn plugin_editor_close(app: AppHandle, slot: String) -> Result<(), String> {
    on_main(&app, move |_| close_on_main(&slot)).await
}

/// Slot ids with an open editor window.
#[tauri::command]
pub async fn plugin_editors_open(app: AppHandle) -> Result<Vec<String>, String> {
    on_main(&app, |_| OPEN.with(|o| o.borrow().values().map(|x| x.slot.clone()).collect::<Vec<_>>())).await
}

/// Linux: plugin editors embed X11 windows, so under Wayland run through XWayland
/// (set before GTK starts; `VCSD_WAYLAND=1` keeps native Wayland, without plugin windows).
pub fn prefer_x11() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("GDK_BACKEND").is_none()
        && std::env::var_os("VCSD_WAYLAND").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_some()
    {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}
