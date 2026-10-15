//! Bug-report snapshots: a hotkey (`[overlay] snapshot_key`, default Home) or the overlay's
//! button saves the last `snapshot_seconds` of logs, packets and timings plus a screenshot
//! into `<session>/snapshots/<time>/`.
//!
//! The screenshot is copied from the swapchain image in the overlay's own command buffer
//! (the loader adds `TRANSFER_SRC` usage to the swapchain), read back once that frame's
//! fence has signalled, and written as PNG on a worker thread.

use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use ash::vk;

static REQUESTED: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Option<(Capture, vk::Fence)>> = Mutex::new(None);

#[derive(Clone)]
pub struct Request {
    pub dir: PathBuf,
}

pub struct Capture {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: [u32; 2],
    bgra: bool,
    request: Request,
}

// Vulkan handles are plain integers; only the present thread uses them.
unsafe impl Send for Capture {}

/// Starts a snapshot (any thread).
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Whether the overlay must draw this frame for a snapshot.
pub fn wants_frame() -> bool {
    REQUESTED.load(Ordering::SeqCst) || PENDING.lock().unwrap().is_some()
}

/// Present thread: takes a pending request and writes everything but the screenshot.
pub fn take_frame_request() -> Option<Request> {
    if !REQUESTED.swap(false, Ordering::SeqCst) {
        return None;
    }
    let dir = crate::host::session_dir()
        .join("snapshots")
        .join(poe_hook_shared::local_timestamp());
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("snapshot: {}: {e}", dir.display());
        return None;
    }
    let seconds = crate::config().overlay.snapshot_seconds.max(1) as u64;
    let since = poe_hook_shared::unix_ms().saturating_sub(seconds * 1000);
    let write_dir = dir.clone();
    crate::hooks::spawn(move || write_sections(&write_dir, since));
    log::info!("snapshot: {}", dir.display());
    Some(Request { dir })
}

fn write_sections(dir: &std::path::Path, since: u64) {
    let mut log = String::new();
    crate::host::logs_since(0, |r| {
        if r.unix_ms >= since {
            log.push_str(&format!(
                "{} {} [{}] {}\n",
                crate::util::clock(r.unix_ms),
                crate::util::level_name(r.level),
                unsafe { r.target.as_str() },
                unsafe { r.message.as_str() }
            ));
        }
    });
    let files: [(&str, String); 4] = [
        ("log.txt", log),
        ("packets.txt", crate::packets::snapshot_text(since)),
        ("timings.csv", crate::timing::snapshot_csv(since)),
        ("state.txt", crate::ui::state_text()),
    ];
    for (name, text) in files {
        if let Err(e) = std::fs::write(dir.join(name), text) {
            log::error!("snapshot: {name}: {e}");
        }
    }
    crate::ui::notice(format!("Snapshot saved: {}", dir.display()));
}

/// Records the copy of the presented image into a host-visible buffer (present thread, inside
/// the overlay's command buffer, after its render pass).
#[allow(clippy::too_many_arguments)]
pub fn record_copy(
    device: &ash::Device,
    memory: &vk::PhysicalDeviceMemoryProperties,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    usage: vk::ImageUsageFlags,
    format: vk::Format,
    [w, h]: [u32; 2],
    request: Request,
) -> Option<Capture> {
    if !usage.contains(vk::ImageUsageFlags::TRANSFER_SRC) {
        log::warn!("snapshot: no screenshot (the swapchain cannot be copied from)");
        return None;
    }
    let size = w as u64 * h as u64 * 4;
    let buffer = unsafe {
        device.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(vk::BufferUsageFlags::TRANSFER_DST),
            None,
        )
    }
    .ok()?;
    let req = unsafe { device.get_buffer_memory_requirements(buffer) };
    let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
    let Some(type_index) = (0..memory.memory_type_count).find(|&i| {
        req.memory_type_bits & (1 << i) != 0
            && memory.memory_types[i as usize]
                .property_flags
                .contains(wanted)
    }) else {
        unsafe { device.destroy_buffer(buffer, None) };
        return None;
    };
    let mem = match unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(type_index),
            None,
        )
    } {
        Ok(m) => m,
        Err(_) => {
            unsafe { device.destroy_buffer(buffer, None) };
            return None;
        }
    };
    unsafe {
        let _ = device.bind_buffer_memory(buffer, mem, 0);
        let to_copy = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(crate::overlay::color_range());
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_copy],
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width: w,
                height: h,
                depth: 1,
            });
        device.cmd_copy_image_to_buffer(
            cmd,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            buffer,
            &[region],
        );
        let back = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(crate::overlay::color_range());
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[back],
        );
    }
    let bgra = matches!(
        format,
        vk::Format::B8G8R8A8_SRGB | vk::Format::B8G8R8A8_UNORM
    );
    Some(Capture {
        buffer,
        memory: mem,
        size: [w, h],
        bgra,
        request,
    })
}

pub fn frame_submitted(capture: Capture, fence: vk::Fence) {
    *PENDING.lock().unwrap() = Some((capture, fence));
}

/// Present thread: reads a finished capture back and hands it to a writer thread.
pub fn poll(device: &ash::Device) {
    let mut pending = PENDING.lock().unwrap();
    let Some((capture, fence)) = pending.as_ref() else {
        return;
    };
    if unsafe { device.get_fence_status(*fence) } != Ok(true) {
        return;
    }
    let [w, h] = capture.size;
    let len = w as usize * h as usize * 4;
    let mut pixels = vec![0u8; len];
    unsafe {
        if let Ok(p) = device.map_memory(capture.memory, 0, len as u64, vk::MemoryMapFlags::empty())
        {
            std::ptr::copy_nonoverlapping(p as *const u8, pixels.as_mut_ptr(), len);
            device.unmap_memory(capture.memory);
        }
        device.destroy_buffer(capture.buffer, None);
        device.free_memory(capture.memory, None);
    }
    let (bgra, path) = (capture.bgra, capture.request.dir.join("screenshot.png"));
    *pending = None;
    crate::hooks::spawn(move || {
        if bgra {
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        for px in pixels.chunks_exact_mut(4) {
            px[3] = 255;
        }
        match write_png(&path, w, h, &pixels) {
            Ok(()) => log::info!("snapshot: screenshot {}", path.display()),
            Err(e) => log::error!("snapshot: screenshot: {e}"),
        }
    });
}

fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}
