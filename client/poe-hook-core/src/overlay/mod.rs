//! In-game debug overlay (Dear ImGui), drawn into the game's Vulkan swapchain.
//!
//! - Window messages arrive on the game's window thread ([`window_message`]): the toggle and
//!   snapshot hotkeys are handled there, other input is queued for imgui and kept from the game
//!   while imgui wants it (mouse over an overlay window, a focused text field).
//! - Frames are built and drawn on one overlay thread: dear-imgui-rs keeps its context and
//!   font-atlas state per thread, and the game presents from several threads. [`present`]
//!   hands the frame to that thread and waits for the semaphore to present with; the game's
//!   present call holds the queue meanwhile, so the overlay's submit is synchronized with it.
//!   While the overlay is hidden nothing is drawn, except the read-only badge and short notices.
//! - [`stop`] (core unload) has the overlay thread release the Vulkan objects and exit.

mod gpu;
mod input;

use std::{
    ffi::c_void,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Receiver, Sender, SyncSender, channel, sync_channel},
    },
    time::{Duration, Instant},
};

use poe_hook_shared::abi::VulkanState;

use ash::vk::{self, Handle};
pub use gpu::color_range;

use crate::ui::App;

static VISIBLE: AtomicBool = AtomicBool::new(false);
static WANT_MOUSE: AtomicBool = AtomicBool::new(false);
static WANT_KEYBOARD: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);
/// Swapchain size, for mapping window coordinates to imgui's.
static EXTENT: (AtomicU32, AtomicU32) = (AtomicU32::new(0), AtomicU32::new(0));
static INPUT: Mutex<Vec<input::Event>> = Mutex::new(Vec::new());
/// Jobs for the overlay thread (started with the first frame).
static WORKER: Mutex<Option<Sender<Job>>> = Mutex::new(None);

enum Job {
    Frame {
        queue: u64,
        image_index: u32,
        wait: Vec<u64>,
        state: VulkanState,
        reply: SyncSender<u64>,
    },
    Release(u64, SyncSender<()>),
    Stop(SyncSender<()>),
}

struct Overlay {
    // Drop order: the GPU objects (and the renderer) before the imgui context.
    gpu: gpu::Gpu,
    plots: dear_implot::PlotContext,
    ctx: imgui::Context,
    app: App,
    last: Instant,
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.gpu.shutdown_renderer(&mut self.ctx);
    }
}

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
    match imgui::StbTrueTypeFontData::from_slice(include_bytes!("../../assets/DejaVuSansMono.ttf"))
    {
        Ok(data) => {
            ctx.font_atlas()
                .add_font(&[imgui::FontSource::stb_truetype_with_size(data, size)]);
        }
        Err(e) => log::warn!("overlay font: {e:?}"),
    }
    ctx.style_mut().scale_all_sizes(cfg.scale);
    ctx
}

/// `vkQueuePresentKHR` callback (any of the game's present threads).
pub fn present(queue: u64, info: *const c_void) -> u64 {
    if STOPPING.load(Ordering::SeqCst) || FAILED.load(Ordering::SeqCst) {
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
    let swapchains =
        unsafe { std::slice::from_raw_parts(info.p_swapchains, info.swapchain_count as usize) };
    let Some(i) = swapchains
        .iter()
        .position(|s| s.as_raw() == state.swapchain)
    else {
        return 0;
    };
    let image_index = unsafe { *info.p_image_indices.add(i) };
    let wait = unsafe {
        std::slice::from_raw_parts(info.p_wait_semaphores, info.wait_semaphore_count as usize)
    }
    .iter()
    .map(|s| s.as_raw())
    .collect();
    let (reply, answer) = sync_channel(1);
    if !send(Job::Frame {
        queue,
        image_index,
        wait,
        state,
        reply,
    }) {
        return 0;
    }
    // Always answered (0 when nothing was submitted): the game's semaphores must be waited on
    // exactly once, by us or by the present.
    answer.recv().unwrap_or(0)
}

fn send(job: Job) -> bool {
    let mut worker = WORKER.lock().unwrap();
    if worker.is_none() {
        if STOPPING.load(Ordering::SeqCst) {
            return false;
        }
        let (tx, rx) = channel();
        crate::hooks::spawn(move || run(rx));
        *worker = Some(tx);
    }
    worker.as_ref().unwrap().send(job).is_ok()
}

/// The overlay thread: owns the imgui context and every Vulkan object of the overlay.
fn run(jobs: Receiver<Job>) {
    let mut overlay: Option<Overlay> = None;
    for job in jobs {
        match job {
            Job::Frame {
                queue,
                image_index,
                wait,
                state,
                reply,
            } => {
                let semaphore = frame(&mut overlay, queue, image_index, &wait, &state);
                let _ = reply.send(semaphore);
                // After the game's present went on: reading a screenshot back takes a while.
                if let Some(o) = overlay.as_ref() {
                    crate::snapshot::poll(o.gpu.device());
                }
            }
            Job::Release(swapchain, done) => {
                if let Some(o) = overlay.as_mut() {
                    o.gpu.release(swapchain);
                }
                let _ = done.send(());
            }
            Job::Stop(done) => {
                overlay.take();
                let _ = done.send(());
                return;
            }
        }
    }
}

fn frame(
    overlay: &mut Option<Overlay>,
    queue: u64,
    image_index: u32,
    wait: &[u64],
    state: &VulkanState,
) -> u64 {
    if FAILED.load(Ordering::SeqCst) {
        return 0;
    }
    if overlay.is_none() {
        match gpu::Gpu::new(state, vk::Queue::from_raw(queue)) {
            Ok(gpu) => {
                let ctx = create_context();
                *overlay = Some(Overlay {
                    gpu,
                    plots: dear_implot::PlotContext::create(&ctx),
                    ctx,
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
    let o = overlay.as_mut().unwrap();
    if let Err(e) = o.gpu.ensure_target(state, &mut o.ctx) {
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
        io.set_display_size([w as f32, h as f32]);
        io.set_delta_time(
            now.duration_since(o.last)
                .max(Duration::from_micros(100))
                .as_secs_f32(),
        );
        for event in std::mem::take(&mut *INPUT.lock().unwrap()) {
            event.apply(io);
        }
    }
    o.last = now;
    let ui = o.ctx.frame();
    o.app.draw(ui, visible(), &o.plots);
    let (want_mouse, want_keyboard) = {
        let io = ui.io();
        (io.want_capture_mouse(), io.want_capture_keyboard())
    };
    WANT_MOUSE.store(want_mouse && visible(), Ordering::Relaxed);
    WANT_KEYBOARD.store(want_keyboard && visible(), Ordering::Relaxed);
    let shot = crate::snapshot::take_frame_request();
    let image = o.gpu.image(image_index);
    let mut capture = None;
    let memory = o.gpu.memory_properties();
    let wait: Vec<vk::Semaphore> = wait.iter().map(|&s| vk::Semaphore::from_raw(s)).collect();
    let result = o.gpu.draw(image_index, &wait, &mut o.ctx, |device, cmd| {
        if let (Some(request), Some((image, usage, format))) = (shot, image) {
            capture = crate::snapshot::record_copy(
                device,
                &memory,
                cmd,
                image,
                usage,
                format,
                [w, h],
                request,
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
            0
        }
    }
}

/// The game is about to destroy a swapchain.
pub fn swapchain_destroy(swapchain: u64) {
    if WORKER.lock().unwrap().is_none() {
        return;
    }
    let (done, wait) = sync_channel(1);
    if send(Job::Release(swapchain, done)) {
        let _ = wait.recv();
    }
}

/// Game window message (window thread). `Some(result)`: consumed.
pub fn window_message(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> Option<isize> {
    input::window_message(hwnd, msg, wparam, lparam)
}

/// Core stop: the overlay thread releases the Vulkan objects and exits.
pub fn stop() {
    STOPPING.store(true, Ordering::SeqCst);
    let Some(tx) = WORKER.lock().unwrap().take() else {
        return;
    };
    let (done, wait) = sync_channel(1);
    if tx.send(Job::Stop(done)).is_ok() {
        let _ = wait.recv_timeout(Duration::from_secs(5));
    }
}
