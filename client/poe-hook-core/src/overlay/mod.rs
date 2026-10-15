//! In-game debug overlay (Dear ImGui), drawn into the game's Vulkan swapchain.
//!
//! - Window messages arrive on the game's window thread ([`window_message`]): the toggle and
//!   snapshot hotkeys are handled there, other input is queued for imgui and kept from the game
//!   while imgui wants it (mouse over an overlay window, a focused text field).
//! - Frames are built and drawn on the game's present thread ([`present`]). While the overlay is
//!   hidden nothing is drawn, except the read-only badge and short notices.
//! - [`stop`] (core unload) asks the present thread to release the Vulkan objects.

mod gpu;
mod input;

use std::{
    ffi::c_void,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use ash::vk::{self, Handle};
pub use gpu::color_range;

use crate::ui::App;

static VISIBLE: AtomicBool = AtomicBool::new(false);
static WANT_MOUSE: AtomicBool = AtomicBool::new(false);
static WANT_KEYBOARD: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);
static RELEASED: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
/// Swapchain size, for mapping window coordinates to imgui's.
static EXTENT: (AtomicU32, AtomicU32) = (AtomicU32::new(0), AtomicU32::new(0));
static INPUT: Mutex<Vec<input::Event>> = Mutex::new(Vec::new());
static OVERLAY: Mutex<Option<Overlay>> = Mutex::new(None);

struct Overlay {
    // Drop order: the GPU objects (and the renderer) before the imgui context.
    gpu: gpu::Gpu,
    ctx: imgui::Context,
    app: App,
    last: Instant,
}

// Only used under the `OVERLAY` mutex.
unsafe impl Send for Overlay {}

pub fn init() {
    VISIBLE.store(crate::config().overlay.visible_at_start, Ordering::SeqCst);
}

pub fn visible() -> bool {
    VISIBLE.load(Ordering::SeqCst)
}

pub fn set_visible(v: bool) {
    VISIBLE.store(v, Ordering::SeqCst);
}

fn create_context() -> imgui::Context {
    let cfg = &crate::config().overlay;
    let mut ctx = imgui::Context::create();
    ctx.set_ini_filename(Some(
        poe_hook_shared::exe_dir()
            .join("logs")
            .join("poe-hook")
            .join("imgui.ini"),
    ));
    let size = (16.0 * cfg.scale).round();
    ctx.fonts().add_font(&[imgui::FontSource::TtfData {
        data: include_bytes!("../../assets/DejaVuSansMono.ttf"),
        size_pixels: size,
        config: Some(imgui::FontConfig {
            oversample_h: 2,
            ..Default::default()
        }),
    }]);
    ctx.style_mut().use_dark_colors();
    ctx.style_mut().window_rounding = 4.0;
    ctx.style_mut().scale_all_sizes(cfg.scale);
    ctx
}

/// `vkQueuePresentKHR` callback (game's present thread).
pub fn present(queue: u64, info: *const c_void) -> u64 {
    if STOPPING.load(Ordering::SeqCst) {
        if !RELEASED.load(Ordering::SeqCst) {
            OVERLAY.lock().unwrap().take();
            RELEASED.store(true, Ordering::SeqCst);
        }
        return 0;
    }
    if FAILED.load(Ordering::SeqCst) {
        return 0;
    }
    let notices = crate::ui::has_notices();
    if !visible() && !notices && !crate::host::read_only() && !crate::snapshot::wants_frame() {
        return 0;
    }
    let info = unsafe { &*(info as *const vk::PresentInfoKHR<'_>) };
    let Some(state) = crate::host::vulkan() else {
        return 0;
    };
    let swapchains = unsafe {
        std::slice::from_raw_parts(info.p_swapchains, info.swapchain_count as usize)
    };
    let Some(i) = swapchains.iter().position(|s| s.as_raw() == state.swapchain) else {
        return 0;
    };
    let image_index = unsafe { *info.p_image_indices.add(i) };
    let wait = unsafe {
        std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize)
    };
    let mut guard = OVERLAY.lock().unwrap();
    if guard.is_none() {
        match gpu::Gpu::new(&state, vk::Queue::from_raw(queue)) {
            Ok(gpu) => {
                *guard = Some(Overlay {
                    gpu,
                    ctx: create_context(),
                    app: App::new(),
                    last: Instant::now(),
                })
            }
            Err(e) => {
                log::error!("overlay disabled: {e}");
                FAILED.store(true, Ordering::SeqCst);
                return 0;
            }
        }
    }
    let o = guard.as_mut().unwrap();
    if let Err(e) = o.gpu.ensure_target(&state, &mut o.ctx) {
        log::error!("overlay disabled: {e}");
        FAILED.store(true, Ordering::SeqCst);
        return 0;
    }
    let [w, h] = o.gpu.extent().unwrap_or([state.width, state.height]);
    EXTENT.0.store(w, Ordering::Relaxed);
    EXTENT.1.store(h, Ordering::Relaxed);
    let now = Instant::now();
    {
        let io = o.ctx.io_mut();
        io.display_size = [w as f32, h as f32];
        io.delta_time = now
            .duration_since(o.last)
            .max(Duration::from_micros(100))
            .as_secs_f32();
        for event in std::mem::take(&mut *INPUT.lock().unwrap()) {
            event.apply(io);
        }
    }
    o.last = now;
    let ui = o.ctx.new_frame();
    o.app.draw(ui, visible());
    let (want_mouse, want_keyboard) = {
        let io = ui.io();
        (io.want_capture_mouse, io.want_capture_keyboard)
    };
    WANT_MOUSE.store(want_mouse && visible(), Ordering::Relaxed);
    WANT_KEYBOARD.store(want_keyboard && visible(), Ordering::Relaxed);
    let draw_data = o.ctx.render();
    let shot = crate::snapshot::take_frame_request();
    if draw_data.total_vtx_count == 0 && shot.is_none() {
        return 0;
    }
    crate::snapshot::poll(o.gpu.device());
    let image = o.gpu.image(image_index);
    let mut capture = None;
    let memory = o.gpu.memory_properties();
    let result = o.gpu.draw(image_index, wait, draw_data, |device, cmd| {
        if let (Some(request), Some((image, usage, format))) = (shot, image) {
            capture = crate::snapshot::record_copy(
                device, &memory, cmd, image, usage, format, [w, h], request,
            );
        }
    });
    match result {
        Ok((semaphore, fence)) => {
            if let Some(c) = capture {
                crate::snapshot::frame_submitted(c, fence);
            }
            semaphore.as_raw()
        }
        Err(e) => {
            log::error!("overlay disabled: {e}");
            FAILED.store(true, Ordering::SeqCst);
            // The game's semaphores may already be consumed by a failed submit; nothing safe
            // to do but present unchanged.
            0
        }
    }
}

/// The game is about to destroy a swapchain.
pub fn swapchain_destroy(swapchain: u64) {
    if let Some(o) = OVERLAY.lock().unwrap().as_mut() {
        o.gpu.release(swapchain);
    }
}

/// Game window message (window thread). `Some(result)`: consumed.
pub fn window_message(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> Option<isize> {
    input::window_message(hwnd, msg, wparam, lparam)
}

/// Core stop: release the Vulkan objects on the present thread (or here if it does not come).
pub fn stop() {
    STOPPING.store(true, Ordering::SeqCst);
    let start = Instant::now();
    while !RELEASED.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !RELEASED.swap(true, Ordering::SeqCst) {
        OVERLAY.lock().unwrap().take();
    }
}
