//! Vulkan side of the overlay: draws imgui into the swapchain image inside the game's
//! `vkQueuePresentKHR` (on the game's present thread, which owns the queue at that moment).
//!
//! Per present: wait for the slot's fence, record a render pass (load, not clear; the image
//! stays in `PRESENT_SRC_KHR` before and after) with the imgui draw data, submit it waiting on
//! the game's present semaphores and signalling the slot's semaphore, which the present then
//! waits on instead. Slots rotate in step with the renderer's per-frame buffers.

use ash::vk::{self, Handle};
use dear_imgui_ash::{AshRenderer, AshRendererConfig, Options, TextureRetirementBatch};
use poe_hook_shared::abi::VulkanState;

/// Frames in flight (command buffers, fences, semaphores; also the renderer's mesh buffers).
const SLOTS: usize = 3;
const FENCE_TIMEOUT_NS: u64 = 2_000_000_000;

struct Slot {
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    done: vk::Semaphore,
    used: bool,
}

/// Resources tied to one swapchain.
struct Target {
    swapchain: u64,
    generation: u64,
    format: vk::Format,
    extent: vk::Extent2D,
    images: Vec<vk::Image>,
    usage: vk::ImageUsageFlags,
    render_pass: vk::RenderPass,
    views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
}

pub struct Gpu {
    device: ash::Device,
    swapchain_fn: ash::khr::swapchain::Device,
    pool: vk::CommandPool,
    slots: Vec<Slot>,
    next: usize,
    target: Option<Target>,
    renderer: Option<AshRenderer>,
    /// Render pass kept across swapchain rebuilds while the format stays the same (the
    /// renderer's pipeline is built for it).
    render_pass: Option<(vk::Format, vk::RenderPass)>,
    /// Texture retirements waiting for the fence of the frame that last used them.
    retirements: Vec<(TextureRetirementBatch, vk::Fence)>,
    queue: vk::Queue,
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
}

pub type Result<T> = std::result::Result<T, String>;

fn vkerr(what: &str) -> impl Fn(vk::Result) -> String + '_ {
    move |e| format!("{what}: {e:?}")
}

impl Gpu {
    /// Loads the device functions and creates the slots (the renderer is created on the first
    /// frame, when the swapchain is known).
    pub fn new(state: &VulkanState, queue: vk::Queue) -> Result<Self> {
        if state.get_instance_proc_addr == 0 || state.instance == 0 || state.device == 0 {
            return Err("Vulkan not ready".into());
        }
        let gipa: vk::PFN_vkGetInstanceProcAddr =
            unsafe { std::mem::transmute(state.get_instance_proc_addr) };
        let entry = unsafe {
            ash::Entry::from_static_fn(ash::StaticFn {
                get_instance_proc_addr: gipa,
            })
        };
        let instance = unsafe {
            ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(state.instance))
        };
        let device =
            unsafe { ash::Device::load(instance.fp_v1_0(), vk::Device::from_raw(state.device)) };
        let swapchain_fn = ash::khr::swapchain::Device::new(&instance, &device);
        let family = crate::host::queue_family(queue.as_raw()).max(0) as u32;
        let pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }
        .map_err(vkerr("command pool"))?;
        let cmds = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(SLOTS as u32),
            )
        }
        .map_err(vkerr("command buffers"))?;
        let mut slots = Vec::new();
        for cmd in cmds {
            let fence = unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }
                .map_err(vkerr("fence"))?;
            let done =
                unsafe { device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None) }
                    .map_err(vkerr("semaphore"))?;
            slots.push(Slot {
                cmd,
                fence,
                done,
                used: false,
            });
        }
        log::info!(
            "overlay: Vulkan ready (queue family {family}, device {:#x})",
            state.device
        );
        Ok(Self {
            device,
            swapchain_fn,
            pool,
            slots,
            next: 0,
            target: None,
            renderer: None,
            render_pass: None,
            retirements: Vec::new(),
            queue,
            instance,
            physical: vk::PhysicalDevice::from_raw(state.physical_device),
        })
    }

    pub fn extent(&self) -> Option<[u32; 2]> {
        self.target
            .as_ref()
            .map(|t| [t.extent.width, t.extent.height])
    }

    /// The swapchain image and its usage (snapshots copy from it).
    pub fn image(&self, index: u32) -> Option<(vk::Image, vk::ImageUsageFlags, vk::Format)> {
        let t = self.target.as_ref()?;
        Some((*t.images.get(index as usize)?, t.usage, t.format))
    }

    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    pub fn memory_properties(&self) -> vk::PhysicalDeviceMemoryProperties {
        unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical)
        }
    }

    /// Makes sure the per-swapchain resources match `state` (recreated after a resize).
    pub fn ensure_target(&mut self, state: &VulkanState, ctx: &mut imgui::Context) -> Result<()> {
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.swapchain == state.swapchain && t.generation == state.generation)
        {
            return Ok(());
        }
        self.wait_idle();
        self.drop_target();
        let format = vk::Format::from_raw(state.format);
        if self.render_pass.is_some_and(|(f, _)| f != format) {
            self.shutdown_renderer(ctx);
        }
        if self.render_pass.is_none() {
            self.render_pass = Some((format, self.create_render_pass(format)?));
        }
        let render_pass = self.render_pass.unwrap().1;
        let target = self.create_target(state, render_pass)?;
        if self.renderer.is_none() {
            let srgb = matches!(
                format,
                vk::Format::B8G8R8A8_SRGB
                    | vk::Format::R8G8B8A8_SRGB
                    | vk::Format::A8B8G8R8_SRGB_PACK32
            );
            let config = AshRendererConfig::with_render_pass(
                self.device.clone(),
                self.queue,
                self.pool,
                render_pass,
            )
            .with_options(Options {
                in_flight_frames: SLOTS,
                framebuffer_srgb: srgb,
                ..Default::default()
            });
            self.renderer = Some(
                unsafe {
                    AshRenderer::with_default_allocator(&self.instance, self.physical, config, ctx)
                }
                .map_err(|e| format!("renderer: {e}"))?,
            );
        }
        log::info!(
            "overlay: target {}x{} {:?}, {} images",
            target.extent.width,
            target.extent.height,
            target.format,
            target.images.len()
        );
        self.target = Some(target);
        Ok(())
    }

    /// Releases the renderer (and the render pass it was built for).
    pub fn shutdown_renderer(&mut self, ctx: &mut imgui::Context) {
        self.wait_idle();
        if let Some(mut r) = self.renderer.take()
            && let Err(e) = r.shutdown(ctx)
        {
            log::warn!("overlay: renderer shutdown: {e}");
        }
        self.retirements.clear();
        if let Some((_, rp)) = self.render_pass.take() {
            unsafe { self.device.destroy_render_pass(rp, None) };
        }
    }

    fn create_render_pass(&self, format: vk::Format) -> Result<vk::RenderPass> {
        let attachment = vk::AttachmentDescription::default()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::LOAD)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
        let color = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color);
        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            );
        let attachments = [attachment];
        let subpasses = [subpass];
        let dependencies = [dependency];
        unsafe {
            self.device.create_render_pass(
                &vk::RenderPassCreateInfo::default()
                    .attachments(&attachments)
                    .subpasses(&subpasses)
                    .dependencies(&dependencies),
                None,
            )
        }
        .map_err(vkerr("render pass"))
    }

    fn create_target(&self, state: &VulkanState, render_pass: vk::RenderPass) -> Result<Target> {
        let swapchain = vk::SwapchainKHR::from_raw(state.swapchain);
        let images = if state.image_count > 0 {
            state.images[..state.image_count as usize]
                .iter()
                .map(|&i| vk::Image::from_raw(i))
                .collect()
        } else {
            unsafe { self.swapchain_fn.get_swapchain_images(swapchain) }
                .map_err(vkerr("swapchain images"))?
        };
        let format = vk::Format::from_raw(state.format);
        let extent = vk::Extent2D {
            width: state.width,
            height: state.height,
        };
        let mut views = Vec::new();
        let mut framebuffers = Vec::new();
        for &image in &images {
            let view = unsafe {
                self.device.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .subresource_range(color_range()),
                    None,
                )
            }
            .map_err(vkerr("image view"))?;
            views.push(view);
            let attachments = [view];
            let fb = unsafe {
                self.device.create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(render_pass)
                        .attachments(&attachments)
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1),
                    None,
                )
            }
            .map_err(vkerr("framebuffer"))?;
            framebuffers.push(fb);
        }
        Ok(Target {
            swapchain: state.swapchain,
            generation: state.generation,
            format,
            extent,
            images,
            usage: vk::ImageUsageFlags::from_raw(state.usage),
            render_pass,
            views,
            framebuffers,
        })
    }

    /// Records and submits one overlay frame; returns the semaphore the present must wait on.
    /// `extra` records more commands after the overlay (snapshot copies).
    pub fn draw(
        &mut self,
        image_index: u32,
        wait: &[vk::Semaphore],
        ctx: &mut imgui::Context,
        extra: impl FnOnce(&ash::Device, vk::CommandBuffer),
    ) -> Result<(vk::Semaphore, vk::Fence)> {
        self.complete_retirements();
        let (Some(target), Some(renderer)) = (&self.target, &mut self.renderer) else {
            return Err("no target".into());
        };
        let framebuffer = *target
            .framebuffers
            .get(image_index as usize)
            .ok_or("image index out of range")?;
        let slot = &mut self.slots[self.next];
        let d = &self.device;
        unsafe {
            if slot.used {
                d.wait_for_fences(&[slot.fence], true, FENCE_TIMEOUT_NS)
                    .map_err(vkerr("wait fence"))?;
            }
            d.reset_fences(&[slot.fence])
                .map_err(vkerr("reset fence"))?;
            d.reset_command_buffer(slot.cmd, vk::CommandBufferResetFlags::empty())
                .map_err(vkerr("reset cmd"))?;
            d.begin_command_buffer(
                slot.cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(vkerr("begin cmd"))?;
            d.cmd_begin_render_pass(
                slot.cmd,
                &vk::RenderPassBeginInfo::default()
                    .render_pass(target.render_pass)
                    .framebuffer(framebuffer)
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent: target.extent,
                    }),
                vk::SubpassContents::INLINE,
            );
            let consumer = renderer
                .renderer_consumer()
                .map_err(|e| format!("imgui consumer: {e}"))?;
            let pending = ctx.render(consumer);
            let (frame, retirement) = renderer
                .prepare_frame(pending)
                .map_err(|e| format!("imgui prepare: {e}"))?;
            if let Some(batch) = retirement {
                self.retirements.push((batch, slot.fence));
            }
            renderer
                .cmd_draw(slot.cmd, frame)
                .map_err(|e| format!("imgui draw: {e}"))?;
            d.cmd_end_render_pass(slot.cmd);
            extra(d, slot.cmd);
            d.end_command_buffer(slot.cmd).map_err(vkerr("end cmd"))?;
            let stages = vec![vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT; wait.len()];
            let cmds = [slot.cmd];
            let signal = [slot.done];
            let submit = vk::SubmitInfo::default()
                .wait_semaphores(wait)
                .wait_dst_stage_mask(&stages)
                .command_buffers(&cmds)
                .signal_semaphores(&signal);
            d.queue_submit(self.queue, &[submit], slot.fence)
                .map_err(vkerr("submit"))?;
        }
        slot.used = true;
        self.next = (self.next + 1) % SLOTS;
        Ok((slot.done, slot.fence))
    }

    /// Releases retired textures whose last frame has finished.
    fn complete_retirements(&mut self) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let device = &self.device;
        self.retirements.retain(|(batch, fence)| {
            if unsafe { device.get_fence_status(*fence) } != Ok(true) {
                return true;
            }
            if let Err(e) =
                unsafe { renderer.complete_texture_retirements_with_fences(*batch, &[*fence]) }
            {
                log::warn!("overlay: texture retirement: {e}");
            }
            false
        });
    }

    /// Waits until no overlay frame is in flight.
    pub fn wait_idle(&self) {
        let fences: Vec<vk::Fence> = self
            .slots
            .iter()
            .filter(|s| s.used)
            .map(|s| s.fence)
            .collect();
        if !fences.is_empty() {
            let _ = unsafe { self.device.wait_for_fences(&fences, true, FENCE_TIMEOUT_NS) };
        }
    }

    /// The game is destroying `swapchain`: release what refers to its images.
    pub fn release(&mut self, swapchain: u64) {
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.swapchain == swapchain)
        {
            self.wait_idle();
            self.drop_target();
        }
    }

    fn drop_target(&mut self) {
        let Some(t) = self.target.take() else { return };
        unsafe {
            for fb in t.framebuffers {
                self.device.destroy_framebuffer(fb, None);
            }
            for v in t.views {
                self.device.destroy_image_view(v, None);
            }
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        self.wait_idle();
        // The renderer is shut down with its imgui context by the owner (`shutdown_renderer`).
        if self.renderer.is_some() {
            log::warn!("overlay: renderer dropped without shutdown");
        }
        self.drop_target();
        if let Some((_, rp)) = self.render_pass.take() {
            unsafe { self.device.destroy_render_pass(rp, None) };
        }
        unsafe {
            for s in &self.slots {
                self.device.destroy_fence(s.fence, None);
                self.device.destroy_semaphore(s.done, None);
            }
            self.device.destroy_command_pool(self.pool, None);
        }
        log::info!("overlay: Vulkan objects released");
    }
}

pub fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}
