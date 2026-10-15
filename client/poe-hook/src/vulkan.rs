//! Vulkan hooks for the overlay.
//!
//! The game loads `vulkan-1.dll` itself and resolves every function through
//! `vkGetInstanceProcAddr` / `vkGetDeviceProcAddr` (Ghidra `0x1410262d0`). Both exports are
//! redirected in the DLL's export table; when the game asks for one of the functions below, it gets our wrapper,
//! which records what the core needs (instance, device, queues, window, swapchain) and calls
//! the real function. `vkQueuePresentKHR` lets the core draw the overlay into the image just
//! before it is shown.
//!
//! Installed from the loader's thread right after `DllMain`, before the game creates its
//! renderer, and never removed (the core is reloadable, these hooks are not).

use std::{
    collections::HashMap,
    ffi::{CStr, c_char, c_void},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use ash::vk::{self, Handle};
use poe_hook_shared::abi::VulkanState;
use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;

/// The real `vkGetInstanceProcAddr` (the export before patching).
static REAL_GIPA: AtomicUsize = AtomicUsize::new(0);

static STATE: Mutex<VulkanState> = Mutex::new(VulkanState {
    present_family: -1,
    ..unsafe { std::mem::zeroed() }
});
static QUEUES: Mutex<Option<HashMap<u64, i32>>> = Mutex::new(None);
static SURFACES: Mutex<Option<HashMap<u64, isize>>> = Mutex::new(None);

/// Wrapped functions: name, slot of the real pointer, wrapper.
macro_rules! wrapped {
    ($($name:literal => $slot:ident, $wrapper:ident;)*) => {
        $(static $slot: AtomicUsize = AtomicUsize::new(0);)*
        /// Our wrapper for `name` (recording the real function `real`), if we wrap it.
        fn wrap(name: &[u8], real: usize) -> Option<usize> {
            match name {
                $($name => {
                    if $slot.swap(real, Ordering::SeqCst) != real {
                        log::debug!("vulkan: wrapping {}", String::from_utf8_lossy(name));
                    }
                    Some($wrapper as *const () as usize)
                })*
                _ => None,
            }
        }
    };
}

wrapped! {
    b"vkCreateInstance" => CREATE_INSTANCE, create_instance;
    b"vkCreateDevice" => CREATE_DEVICE, create_device;
    b"vkGetDeviceQueue" => GET_DEVICE_QUEUE, get_device_queue;
    b"vkGetDeviceQueue2" => GET_DEVICE_QUEUE2, get_device_queue2;
    b"vkCreateWin32SurfaceKHR" => CREATE_SURFACE, create_win32_surface;
    b"vkGetPhysicalDeviceSurfaceSupportKHR" => SURFACE_SUPPORT, surface_support;
    b"vkGetPhysicalDeviceSurfaceCapabilitiesKHR" => SURFACE_CAPS, surface_caps;
    b"vkCreateSwapchainKHR" => CREATE_SWAPCHAIN, create_swapchain;
    b"vkDestroySwapchainKHR" => DESTROY_SWAPCHAIN, destroy_swapchain;
    b"vkQueuePresentKHR" => QUEUE_PRESENT, queue_present;
    b"vkGetDeviceProcAddr" => GET_DEVICE_PROC_ADDR, get_device_proc_addr;
    // Keep the game on our resolver if it asks for it again.
    b"vkGetInstanceProcAddr" => INSTANCE_PROC_ADDR_AGAIN, get_instance_proc_addr;
}

pub fn init() -> Result<(), String> {
    let name: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
    // The game loads it later itself; loading it first only adds a reference.
    let module = unsafe { LoadLibraryW(name.as_ptr()) };
    if module.is_null() {
        return Err("vulkan-1.dll not found (no overlay)".into());
    }
    // Export-table patches, not inline hooks: Wine's exports are tiny thunks that a 14-byte
    // jump would overrun. `GetProcAddress` then hands the game our functions.
    let real = patch_export(
        module as usize,
        b"vkGetInstanceProcAddr",
        get_instance_proc_addr as *const () as usize,
    )?;
    REAL_GIPA.store(real, Ordering::SeqCst);
    let real = patch_export(
        module as usize,
        b"vkGetDeviceProcAddr",
        get_device_proc_addr as *const () as usize,
    )?;
    GET_DEVICE_PROC_ADDR.store(real, Ordering::SeqCst);
    Ok(())
}

/// Points the export `name` of the module at `base` to `target` (through a jump stub within
/// 4 GB of the module, as export entries are 32-bit RVAs). Returns the original address.
fn patch_export(base: usize, name: &[u8], target: usize) -> Result<usize, String> {
    use windows_sys::Win32::System::{
        Diagnostics::Debug::IMAGE_NT_HEADERS64,
        Memory::{PAGE_READWRITE, VirtualProtect},
        SystemServices::{IMAGE_DOS_HEADER, IMAGE_EXPORT_DIRECTORY},
    };
    unsafe {
        let dos = &*(base as *const IMAGE_DOS_HEADER);
        let nt = &*((base + dos.e_lfanew as usize) as *const IMAGE_NT_HEADERS64);
        let dir = nt.OptionalHeader.DataDirectory[0];
        let exports = &*((base + dir.VirtualAddress as usize) as *const IMAGE_EXPORT_DIRECTORY);
        let names = (base + exports.AddressOfNames as usize) as *const u32;
        let ordinals = (base + exports.AddressOfNameOrdinals as usize) as *const u16;
        let functions = (base + exports.AddressOfFunctions as usize) as *mut u32;
        for i in 0..exports.NumberOfNames as usize {
            let n = CStr::from_ptr((base + *names.add(i) as usize) as *const c_char);
            if n.to_bytes() != name {
                continue;
            }
            let slot = functions.add(*ordinals.add(i) as usize);
            let mut original = base + *slot as usize;
            // A forwarder ("winevulkan.vkGetInstanceProcAddr" under Wine) points into the export
            // directory: resolve it through the target module.
            let range =
                base + dir.VirtualAddress as usize..base + (dir.VirtualAddress + dir.Size) as usize;
            if range.contains(&original) {
                let text = CStr::from_ptr(original as *const c_char)
                    .to_string_lossy()
                    .into_owned();
                let (dll, func) = text
                    .split_once('.')
                    .ok_or(format!("bad forwarder {text}"))?;
                let wide: Vec<u16> = format!("{dll}.dll\0").encode_utf16().collect();
                let target_module = LoadLibraryW(wide.as_ptr());
                let func = std::ffi::CString::new(func).unwrap();
                original = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
                    target_module,
                    func.as_ptr() as *const u8,
                )
                .ok_or(format!("forwarder {text} unresolved"))? as usize;
                log::info!("vulkan: {text} -> {original:#x}");
            }
            let stub = near_stub(base, target)?;
            let rva = u32::try_from(stub - base).map_err(|_| "stub out of range".to_string())?;
            let mut old = 0;
            VirtualProtect(slot as *const _, 4, PAGE_READWRITE, &mut old);
            *slot = rva;
            VirtualProtect(slot as *const _, 4, old, &mut old);
            log::info!(
                "vulkan: export {} {:#x} -> stub {stub:#x}",
                String::from_utf8_lossy(name),
                original
            );
            return Ok(original);
        }
    }
    Err(format!("{} not exported", String::from_utf8_lossy(name)))
}

/// A `jmp [rip]; dq target` stub in executable memory above `base` (within 2 GB).
fn near_stub(base: usize, target: usize) -> Result<usize, String> {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc,
    };
    let mut at = (base + 0x10_0000) & !0xffff;
    while at < base + 0x7000_0000 {
        let p = unsafe {
            VirtualAlloc(
                at as *const _,
                0x1000,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as usize;
        if p != 0 {
            let code: [u8; 6] = [0xff, 0x25, 0, 0, 0, 0];
            unsafe {
                std::ptr::copy_nonoverlapping(code.as_ptr(), p as *mut u8, 6);
                ((p + 6) as *mut usize).write_unaligned(target);
            }
            return Ok(p);
        }
        at += 0x1_0000;
    }
    Err("no memory near vulkan-1.dll for a stub".into())
}

pub extern "C" fn host_vulkan(out: *mut VulkanState) -> bool {
    let mut state = *STATE.lock().unwrap();
    state.get_instance_proc_addr = REAL_GIPA.load(Ordering::SeqCst);
    unsafe { out.write(state) };
    state.swapchain != 0
}

pub extern "C" fn host_queue_family(queue: u64) -> i32 {
    QUEUES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&queue).copied())
        .unwrap_or_else(|| STATE.lock().unwrap().present_family)
}

/// A device function resolved without our wrappers.
fn real_device_proc(device: vk::Device, name: &CStr) -> vk::PFN_vkVoidFunction {
    let instance = STATE.lock().unwrap().instance;
    if REAL_GIPA.load(Ordering::SeqCst) == 0 || instance == 0 {
        return None;
    }
    let gipa: vk::PFN_vkGetInstanceProcAddr = real(&REAL_GIPA);
    let gdpa = unsafe {
        gipa(
            vk::Instance::from_raw(instance),
            c"vkGetDeviceProcAddr".as_ptr(),
        )
    }?;
    let gdpa: vk::PFN_vkGetDeviceProcAddr = unsafe { std::mem::transmute(gdpa) };
    unsafe { gdpa(device, name.as_ptr()) }
}

fn real<T: Copy>(slot: &AtomicUsize) -> T {
    let p = slot.load(Ordering::SeqCst);
    debug_assert!(p != 0);
    unsafe { std::mem::transmute_copy(&p) }
}

/// Resolves a function name, wrapping it when it is one of ours.
fn resolve(name: *const c_char, real_ptr: usize) -> usize {
    if real_ptr == 0 || name.is_null() {
        return real_ptr;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    wrap(name, real_ptr).unwrap_or(real_ptr)
}

unsafe extern "system" fn get_instance_proc_addr(
    instance: vk::Instance,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    let f: vk::PFN_vkGetInstanceProcAddr = real(&REAL_GIPA);
    let p = unsafe { f(instance, name) }.map_or(0, |f| f as usize);
    let p = resolve(name, p);
    unsafe { std::mem::transmute::<usize, vk::PFN_vkVoidFunction>(p) }
}

/// `vkGetDeviceProcAddr` (the export, and as returned by `vkGetInstanceProcAddr`).
unsafe extern "system" fn get_device_proc_addr(
    device: vk::Device,
    name: *const c_char,
) -> vk::PFN_vkVoidFunction {
    let f: vk::PFN_vkGetDeviceProcAddr = real(&GET_DEVICE_PROC_ADDR);
    let p = unsafe { f(device, name) }.map_or(0, |f| f as usize);
    let p = resolve(name, p);
    unsafe { std::mem::transmute::<usize, vk::PFN_vkVoidFunction>(p) }
}

unsafe extern "system" fn create_instance(
    info: *const vk::InstanceCreateInfo<'_>,
    alloc: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::Instance,
) -> vk::Result {
    let f: vk::PFN_vkCreateInstance = real(&CREATE_INSTANCE);
    let result = unsafe { f(info, alloc, out) };
    if result == vk::Result::SUCCESS {
        let instance = unsafe { *out }.as_raw();
        STATE.lock().unwrap().instance = instance;
        log::info!("vulkan: instance {instance:#x}");
    }
    result
}

unsafe extern "system" fn create_device(
    physical: vk::PhysicalDevice,
    info: *const vk::DeviceCreateInfo<'_>,
    alloc: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::Device,
) -> vk::Result {
    let f: vk::PFN_vkCreateDevice = real(&CREATE_DEVICE);
    let result = unsafe { f(physical, info, alloc, out) };
    if result == vk::Result::SUCCESS {
        let device = unsafe { *out }.as_raw();
        let mut s = STATE.lock().unwrap();
        s.physical_device = physical.as_raw();
        s.device = device;
        log::info!(
            "vulkan: device {device:#x} on physical device {:#x}",
            physical.as_raw()
        );
    }
    result
}

fn record_queue(queue: vk::Queue, family: u32) {
    if queue != vk::Queue::null() {
        QUEUES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(queue.as_raw(), family as i32);
    }
}

unsafe extern "system" fn get_device_queue(
    device: vk::Device,
    family: u32,
    index: u32,
    out: *mut vk::Queue,
) {
    let f: vk::PFN_vkGetDeviceQueue = real(&GET_DEVICE_QUEUE);
    unsafe { f(device, family, index, out) };
    record_queue(unsafe { *out }, family);
}

unsafe extern "system" fn get_device_queue2(
    device: vk::Device,
    info: *const vk::DeviceQueueInfo2<'_>,
    out: *mut vk::Queue,
) {
    let f: vk::PFN_vkGetDeviceQueue2 = real(&GET_DEVICE_QUEUE2);
    unsafe { f(device, info, out) };
    record_queue(unsafe { *out }, unsafe { (*info).queue_family_index });
}

unsafe extern "system" fn create_win32_surface(
    instance: vk::Instance,
    info: *const vk::Win32SurfaceCreateInfoKHR<'_>,
    alloc: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::SurfaceKHR,
) -> vk::Result {
    let f: vk::PFN_vkCreateWin32SurfaceKHR = real(&CREATE_SURFACE);
    let result = unsafe { f(instance, info, alloc, out) };
    if result == vk::Result::SUCCESS {
        // The game may have created its instance before the export was patched: the
        // surface's instance is the one in use.
        STATE.lock().unwrap().instance = instance.as_raw();
        let hwnd = unsafe { (*info).hwnd } as isize;
        SURFACES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(unsafe { *out }.as_raw(), hwnd);
        log::info!("vulkan: surface for window {hwnd:#x}");
    }
    result
}

/// Present-support query (the game asks before creating its swapchain): records the
/// physical device and a queue family that can present.
unsafe extern "system" fn surface_support(
    physical: vk::PhysicalDevice,
    family: u32,
    surface: vk::SurfaceKHR,
    out: *mut vk::Bool32,
) -> vk::Result {
    let f: vk::PFN_vkGetPhysicalDeviceSurfaceSupportKHR = real(&SURFACE_SUPPORT);
    let result = unsafe { f(physical, family, surface, out) };
    if result == vk::Result::SUCCESS {
        let mut s = STATE.lock().unwrap();
        s.physical_device = physical.as_raw();
        if unsafe { *out } != 0 && s.present_family < 0 {
            s.present_family = family as i32;
        }
    }
    result
}

unsafe extern "system" fn surface_caps(
    physical: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
    out: *mut vk::SurfaceCapabilitiesKHR,
) -> vk::Result {
    let f: vk::PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR = real(&SURFACE_CAPS);
    STATE.lock().unwrap().physical_device = physical.as_raw();
    unsafe { f(physical, surface, out) }
}

/// Usage flags the surface supports (0 if they cannot be queried).
fn supported_usage(surface: vk::SurfaceKHR) -> vk::ImageUsageFlags {
    let (instance, physical) = {
        let s = STATE.lock().unwrap();
        (s.instance, s.physical_device)
    };
    if instance == 0 || physical == 0 {
        return vk::ImageUsageFlags::empty();
    }
    let gipa: vk::PFN_vkGetInstanceProcAddr = real(&REAL_GIPA);
    let Some(f) = (unsafe {
        gipa(
            vk::Instance::from_raw(instance),
            c"vkGetPhysicalDeviceSurfaceCapabilitiesKHR".as_ptr(),
        )
    }) else {
        return vk::ImageUsageFlags::empty();
    };
    let f: vk::PFN_vkGetPhysicalDeviceSurfaceCapabilitiesKHR = unsafe { std::mem::transmute(f) };
    let mut caps = vk::SurfaceCapabilitiesKHR::default();
    match unsafe { f(vk::PhysicalDevice::from_raw(physical), surface, &mut caps) } {
        vk::Result::SUCCESS => caps.supported_usage_flags,
        _ => vk::ImageUsageFlags::empty(),
    }
}

unsafe extern "system" fn create_swapchain(
    device: vk::Device,
    info: *const vk::SwapchainCreateInfoKHR<'_>,
    alloc: *const vk::AllocationCallbacks<'_>,
    out: *mut vk::SwapchainKHR,
) -> vk::Result {
    let f: vk::PFN_vkCreateSwapchainKHR = real(&CREATE_SWAPCHAIN);
    let mut info = unsafe { *info };
    // The overlay renders into the images; snapshots copy them.
    let supported = supported_usage(info.surface);
    let wanted = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC;
    info.image_usage |= wanted & supported;
    let result = unsafe { f(device, &info, alloc, out) };
    if result != vk::Result::SUCCESS {
        return result;
    }
    let swapchain = unsafe { *out };
    let hwnd = SURFACES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&info.surface.as_raw()).copied())
        .unwrap_or(0);
    let mut images = [0u64; 16];
    let mut count = 0u32;
    if let Some(get) = real_device_proc(device, c"vkGetSwapchainImagesKHR") {
        let get: vk::PFN_vkGetSwapchainImagesKHR = unsafe { std::mem::transmute(get) };
        let mut raw = [vk::Image::null(); 16];
        let _ = unsafe { get(device, swapchain, &mut count, std::ptr::null_mut()) };
        count = count.min(16);
        let _ = unsafe { get(device, swapchain, &mut count, raw.as_mut_ptr()) };
        for (dst, src) in images.iter_mut().zip(raw) {
            *dst = src.as_raw();
        }
    }
    {
        let mut s = STATE.lock().unwrap();
        s.generation += 1;
        s.device = device.as_raw();
        s.swapchain = swapchain.as_raw();
        s.format = info.image_format.as_raw();
        s.width = info.image_extent.width;
        s.height = info.image_extent.height;
        s.usage = info.image_usage.as_raw();
        s.image_count = count;
        s.images = images;
        if hwnd != 0 {
            s.hwnd = hwnd;
        }
    }
    log::info!(
        "vulkan: swapchain {:#x} {}x{} format {:?} usage {:?} images {count}",
        swapchain.as_raw(),
        info.image_extent.width,
        info.image_extent.height,
        info.image_format,
        info.image_usage
    );
    if hwnd != 0 {
        crate::window::install(hwnd);
    }
    result
}

unsafe extern "system" fn destroy_swapchain(
    device: vk::Device,
    swapchain: vk::SwapchainKHR,
    alloc: *const vk::AllocationCallbacks<'_>,
) {
    crate::core::swapchain_destroy(swapchain.as_raw());
    let f: vk::PFN_vkDestroySwapchainKHR = real(&DESTROY_SWAPCHAIN);
    unsafe { f(device, swapchain, alloc) };
    let mut s = STATE.lock().unwrap();
    if s.swapchain == swapchain.as_raw() {
        s.swapchain = 0;
        s.generation += 1;
    }
}

unsafe extern "system" fn queue_present(
    queue: vk::Queue,
    info: *const vk::PresentInfoKHR<'_>,
) -> vk::Result {
    let f: vk::PFN_vkQueuePresentKHR = real(&QUEUE_PRESENT);
    let wait = crate::core::present(queue.as_raw(), info as *const c_void);
    if wait == 0 {
        return unsafe { f(queue, info) };
    }
    let semaphores = [vk::Semaphore::from_raw(wait)];
    let mut copy = unsafe { *info };
    copy.wait_semaphore_count = 1;
    copy.p_wait_semaphores = semaphores.as_ptr();
    unsafe { f(queue, &copy) }
}
