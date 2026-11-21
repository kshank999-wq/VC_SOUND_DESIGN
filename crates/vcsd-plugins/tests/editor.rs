//! A plugin's own editor embedded in a native (X11) window, with the host's
//! frame and run loop. Needs an X display (CI runs it under Xvfb); skipped without one.
#![cfg(target_os = "linux")]
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use vcsd_plugins::vst3host::{scan_factory, Vst3Instance};
use vcsd_plugins::{EditorHost, GesturePhase, Instance, ParentWindow, RunLoop, Setup};
use vst3::ComPtr;
use x11::xlib;

type Callback = Rc<dyn Fn()>;

/// A run loop driven by hand: `pump` polls the watched fds and fires timers once.
#[derive(Default)]
struct TestLoop {
    next: RefCell<u64>,
    fds: RefCell<HashMap<u64, (i32, Callback)>>,
    timers: RefCell<HashMap<u64, Callback>>,
}

impl RunLoop for TestLoop {
    fn watch_fd(&self, fd: i32, f: Box<dyn Fn()>) -> u64 {
        *self.next.borrow_mut() += 1;
        let id = *self.next.borrow();
        self.fds.borrow_mut().insert(id, (fd, f.into()));
        id
    }
    fn every(&self, _ms: u32, f: Box<dyn Fn()>) -> u64 {
        *self.next.borrow_mut() += 1;
        let id = *self.next.borrow();
        self.timers.borrow_mut().insert(id, f.into());
        id
    }
    fn cancel(&self, id: u64) {
        self.fds.borrow_mut().remove(&id);
        self.timers.borrow_mut().remove(&id);
    }
}

impl TestLoop {
    fn pump(&self) {
        let fds: Vec<(i32, Callback)> = self.fds.borrow().values().cloned().collect();
        for (fd, f) in fds {
            let mut p = libc_pollfd { fd, events: 1, revents: 0 };
            if unsafe { poll(&mut p, 1, 200) } > 0 {
                f();
            }
        }
        let ts: Vec<Callback> = self.timers.borrow().values().cloned().collect();
        ts.iter().for_each(|f| f());
    }
}

#[repr(C)]
struct libc_pollfd {
    fd: i32,
    events: i16,
    revents: i16,
}
extern "C" {
    fn poll(fds: *mut libc_pollfd, n: u64, timeout: i32) -> i32;
}

struct Host {
    dpy: *mut xlib::Display,
    win: xlib::Window,
    sizes: RefCell<Vec<(u32, u32)>>,
    rl: Rc<TestLoop>,
}

impl EditorHost for Host {
    fn resize(&self, w: u32, h: u32) -> bool {
        self.sizes.borrow_mut().push((w, h));
        unsafe {
            xlib::XResizeWindow(self.dpy, self.win, w, h);
            xlib::XFlush(self.dpy);
        }
        true
    }
    fn run_loop(&self) -> Option<Rc<dyn RunLoop>> {
        Some(self.rl.clone())
    }
}

#[test]
fn embeds_resizes_and_edits_through_the_run_loop() {
    let dpy = unsafe { xlib::XOpenDisplay(std::ptr::null()) };
    if dpy.is_null() {
        eprintln!("skipped: no X display (run under xvfb-run)");
        return;
    }
    let f = unsafe { ComPtr::from_raw(vcsd_testplug::GetPluginFactory()) }.unwrap();
    let info = scan_factory(&f, "-").unwrap().into_iter().find(|i| i.name == "VCSD Test Gain").unwrap();
    let inst = unsafe { Vst3Instance::from_factory(&info, Setup::default(), vcsd_testplug::GetPluginFactory()) }.unwrap();

    let win = unsafe {
        let root = xlib::XDefaultRootWindow(dpy);
        let w = xlib::XCreateSimpleWindow(dpy, root, 0, 0, 300, 100, 0, 0, 0);
        xlib::XMapWindow(dpy, w);
        xlib::XFlush(dpy);
        w
    };
    let rl = Rc::new(TestLoop::default());
    let host = Rc::new(Host { dpy, win, sizes: RefCell::new(vec![]), rl: rl.clone() });
    let ed = inst.open_editor(ParentWindow::X11(win), host.clone()).unwrap();

    // the plugin asked for a new size through IPlugFrame; the view took it
    assert_eq!(*host.sizes.borrow(), vec![(360, 120)]);
    assert_eq!(ed.size(), (360, 120));
    assert!(ed.resizable());
    assert_eq!(ed.set_size(500, 20), (500, 60), "the plugin's size constraint applies");
    assert_eq!(rl.fds.borrow().len(), 1, "editor watches its X connection");
    assert_eq!(rl.timers.borrow().len(), 1);

    // click 3/4 of the way across the plugin's own window (a child of ours)
    let child = unsafe {
        let (mut r, mut p, mut kids, mut n) = (0, 0, std::ptr::null_mut(), 0);
        xlib::XQueryTree(dpy, win, &mut r, &mut p, &mut kids, &mut n);
        assert_eq!(n, 1, "the plugin embedded one window in ours");
        let c = *kids;
        xlib::XFree(kids as *mut _);
        c
    };
    unsafe {
        let mut ev: xlib::XEvent = std::mem::zeroed();
        ev.button = xlib::XButtonEvent {
            type_: xlib::ButtonPress,
            serial: 0,
            send_event: 1,
            display: dpy,
            window: child,
            root: xlib::XDefaultRootWindow(dpy),
            subwindow: 0,
            time: 0,
            x: 375,
            y: 30,
            x_root: 375,
            y_root: 30,
            state: 0,
            button: 1,
            same_screen: 1,
        };
        xlib::XSendEvent(dpy, child, 0, xlib::ButtonPressMask, &mut ev);
        xlib::XFlush(dpy);
    }
    assert!(!inst.take_dirty());
    for _ in 0..5 {
        rl.pump();
        if inst.take_dirty() {
            break;
        }
    }
    // the touch arrives as a gesture (for writing automation): begin, the value, end
    let gs = inst.take_gestures();
    let phases: Vec<GesturePhase> = gs.iter().map(|g| g.phase).collect();
    assert_eq!(phases, [GesturePhase::Begin, GesturePhase::Move, GesturePhase::End]);
    assert!(gs[0].value.is_nan(), "nothing known before the first move");
    assert!((gs[1].value - 0.75).abs() < 1e-9 && (gs[2].value - 0.75).abs() < 1e-9);
    assert!(inst.take_gestures().is_empty());
    let v = inst.params()[0].value;
    assert!((v - 0.75).abs() < 1e-9, "edit made in the plugin's window reached the controller: {v}");
    let mut buf = vec![0.5f32; 64];
    inst.process(&mut buf, 2, 0, &[]);
    assert!((buf[0] - 0.75).abs() < 1e-6, "…and the processor (gain ×1.5): {}", buf[0]);

    drop(ed);
    assert!(rl.fds.borrow().is_empty() && rl.timers.borrow().is_empty(), "closing unregisters everything");
    unsafe {
        xlib::XDestroyWindow(dpy, win);
        xlib::XCloseDisplay(dpy);
    }
}
