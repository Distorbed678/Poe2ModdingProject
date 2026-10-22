//! The host side of the server mod ABI (`poe-mod-api`): the [`ServerApi`] function table a mod
//! gets, and the per-callback scope behind `submit`.
//!
//! Everything a mod passes in is untrusted: pointers are checked for null, strings for UTF-8 and
//! length, structs are copied by their own `size`. Nothing here panics on bad input.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::c_void,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use poe_mod_api::{
    COMMAND_BROADCAST, COMMAND_KICK, COMMAND_SEND_CHAT, COMMAND_TELEPORT, Command, CommandSpec,
    ERR_BAD_ARGUMENT, ERR_EXISTS, ERR_LIMIT, ERR_NOT_ALLOWED, ERR_NOT_FOUND, LOG_DEBUG, LOG_ERROR,
    LOG_INFO, LOG_TRACE, LOG_WARN, OK, ROLE_ADMIN, ROLE_PLAYER, Str,
};

/// Most chat commands one mod may register.
pub const MAX_COMMANDS_PER_MOD: usize = 32;
/// Most commands one callback may submit.
pub const MAX_SUBMITTED: usize = 64;
/// Longest text of a submitted command (bytes).
pub const MAX_TEXT: usize = 4000;
/// Longest log message (bytes; longer ones are cut).
const MAX_LOG: usize = 8000;

/// A change a mod asked for, validated. Applied by the owner of the thing it changes.
#[derive(Debug, Clone, PartialEq)]
pub enum ModCommand {
    /// A server message to one player (`target` is the player's id of the snapshot).
    SendChat { target: u64, text: String },
    /// A server message to everybody online.
    Broadcast { text: String },
    /// Move the player to a grid cell of their current instance.
    Teleport { target: u64, x: f64, y: f64 },
    /// Disconnect the player; `reason` is shown to them first.
    Kick { target: u64, reason: String },
}

/// A chat command a mod registered with `register_command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCommand {
    pub name: String,
    pub help: String,
    pub admin: bool,
}

/// Per-mod state behind `ServerApi::ctx`. Leaked: a mod may keep the table for the process.
pub struct ModCtx {
    pub id: String,
    pub config: toml::Table,
    /// `register_command` is only allowed while `poe_server_mod_init` runs.
    pub init_phase: AtomicBool,
    pub commands: Mutex<Vec<NativeCommand>>,
    /// Command names taken by the server and by every mod loaded so far (shared).
    pub taken: Arc<Mutex<HashSet<String>>>,
}

/// What `submit` collects during one callback.
pub struct Scope {
    pub ctx: *const ModCtx,
    pub queue: RefCell<Vec<ModCommand>>,
}

thread_local! {
    static CURRENT: Cell<*const Scope> = const { Cell::new(std::ptr::null()) };
}

/// Makes `scope` the one `submit` accepts on this thread until the guard drops.
pub struct ScopeGuard {
    previous: *const Scope,
}

impl ScopeGuard {
    pub fn enter(scope: &Scope) -> Self {
        let previous = CURRENT.with(|c| c.replace(scope));
        Self { previous }
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.previous));
    }
}

fn ctx_of<'a>(ctx: *const c_void) -> Option<&'a ModCtx> {
    // SAFETY: `ctx` is the pointer the host put in the table: a leaked `ModCtx`. A mod that
    // passes anything else breaks the ABI contract; null is the only mistake we can catch.
    unsafe { (ctx as *const ModCtx).as_ref() }
}

/// A string from a mod, checked: null with a length, too long, or not UTF-8 gives `None`.
pub fn read_str(s: Str, max: usize) -> Option<String> {
    if s.len == 0 {
        return Some(String::new());
    }
    if s.ptr.is_null() || s.len > max {
        return None;
    }
    // SAFETY: the mod promises `ptr` is valid for `len` bytes for the call (ABI contract); we
    // only read them and validate the UTF-8 ourselves.
    let bytes = unsafe { std::slice::from_raw_parts(s.ptr, s.len) };
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

pub extern "C" fn api_log(ctx: *const c_void, level: u32, message: Str) {
    let Some(ctx) = ctx_of(ctx) else { return };
    let text = if message.len == 0 || message.ptr.is_null() {
        String::new()
    } else {
        // SAFETY: as in `read_str`; lossy conversion because a log line is best effort.
        let bytes = unsafe { std::slice::from_raw_parts(message.ptr, message.len.min(MAX_LOG)) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    let id = ctx.id.as_str();
    match level {
        LOG_ERROR => tracing::error!(target: "mod", mod_id = id, "{text}"),
        LOG_WARN => tracing::warn!(target: "mod", mod_id = id, "{text}"),
        LOG_INFO => tracing::info!(target: "mod", mod_id = id, "{text}"),
        LOG_DEBUG => tracing::debug!(target: "mod", mod_id = id, "{text}"),
        LOG_TRACE => tracing::trace!(target: "mod", mod_id = id, "{text}"),
        _ => tracing::info!(target: "mod", mod_id = id, "{text}"),
    }
}

/// A config value as text: strings as they are, everything else as TOML renders it.
fn render(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub extern "C" fn api_config_get(ctx: *const c_void, key: Str, buf: *mut u8, cap: usize) -> isize {
    let Some(ctx) = ctx_of(ctx) else {
        return ERR_BAD_ARGUMENT as isize;
    };
    let Some(key) = read_str(key, 256) else {
        return ERR_BAD_ARGUMENT as isize;
    };
    let Some(value) = ctx.config.get(&key) else {
        return ERR_NOT_FOUND as isize;
    };
    let text = render(value);
    let bytes = text.as_bytes();
    if !buf.is_null() && cap > 0 {
        let n = bytes.len().min(cap);
        // SAFETY: the mod gave a writable buffer of `cap` bytes; `n <= cap`.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n) };
    }
    bytes.len() as isize
}

pub extern "C" fn api_register_command(ctx: *const c_void, spec: *const CommandSpec) -> i32 {
    let Some(ctx) = ctx_of(ctx) else {
        return ERR_BAD_ARGUMENT;
    };
    if !ctx.init_phase.load(Ordering::SeqCst) {
        return ERR_NOT_ALLOWED;
    }
    if spec.is_null() {
        return ERR_BAD_ARGUMENT;
    }
    // Copy by the mod's own size: an older mod's struct is shorter than ours.
    // SAFETY: non-null, and `size` is the first field of every versioned struct.
    let size = unsafe { (*spec).size } as usize;
    if size < std::mem::offset_of!(CommandSpec, help) + std::mem::size_of::<Str>() {
        return ERR_BAD_ARGUMENT;
    }
    let mut copy = CommandSpec {
        size: 0,
        role: 0,
        name: Str::EMPTY,
        help: Str::EMPTY,
    };
    let n = size.min(std::mem::size_of::<CommandSpec>());
    // SAFETY: both are at least `n` bytes; the mod promised `size` readable bytes.
    unsafe { std::ptr::copy_nonoverlapping(spec as *const u8, &mut copy as *mut _ as *mut u8, n) };
    let (Some(name), Some(help)) = (read_str(copy.name, 64), read_str(copy.help, 400)) else {
        return ERR_BAD_ARGUMENT;
    };
    if !poe_hook_mods::valid_command_name(&name)
        || help.chars().count() > 200
        || help.contains(['\n', '\t'])
    {
        return ERR_BAD_ARGUMENT;
    }
    let admin = match copy.role {
        ROLE_PLAYER => false,
        ROLE_ADMIN => true,
        _ => return ERR_BAD_ARGUMENT,
    };
    let mut mine = ctx.commands.lock().unwrap();
    if mine.len() >= MAX_COMMANDS_PER_MOD {
        return ERR_LIMIT;
    }
    if !ctx.taken.lock().unwrap().insert(name.clone()) {
        return ERR_EXISTS;
    }
    mine.push(NativeCommand { name, help, admin });
    OK
}

/// Validates a command the mod built (copied by its own `size`).
fn read_command(command: *const Command) -> Result<ModCommand, i32> {
    if command.is_null() {
        return Err(ERR_BAD_ARGUMENT);
    }
    // SAFETY: non-null; `size` is the first field.
    let size = unsafe { (*command).size } as usize;
    // Up to and including `kind` must be there.
    if size < std::mem::offset_of!(Command, target) {
        return Err(ERR_BAD_ARGUMENT);
    }
    let mut c = Command::new(0);
    let n = size.min(std::mem::size_of::<Command>());
    // SAFETY: as in `api_register_command`.
    unsafe { std::ptr::copy_nonoverlapping(command as *const u8, &mut c as *mut _ as *mut u8, n) };
    let text = read_str(c.text, MAX_TEXT).ok_or(ERR_BAD_ARGUMENT)?;
    match c.kind {
        COMMAND_SEND_CHAT if c.target != 0 && !text.is_empty() => Ok(ModCommand::SendChat {
            target: c.target,
            text,
        }),
        COMMAND_BROADCAST if !text.is_empty() => Ok(ModCommand::Broadcast { text }),
        COMMAND_TELEPORT if c.target != 0 && c.x.is_finite() && c.y.is_finite() => {
            Ok(ModCommand::Teleport {
                target: c.target,
                x: c.x,
                y: c.y,
            })
        }
        COMMAND_KICK if c.target != 0 => Ok(ModCommand::Kick {
            target: c.target,
            reason: text,
        }),
        _ => Err(ERR_BAD_ARGUMENT),
    }
}

pub extern "C" fn api_submit(
    ctx: *const c_void,
    scope: *const c_void,
    command: *const Command,
) -> i32 {
    let current = CURRENT.with(Cell::get);
    // Only inside a callback of this thread, with the scope of that very callback.
    if current.is_null() || current as *const c_void != scope {
        return ERR_NOT_ALLOWED;
    }
    // SAFETY: `current` was set by `ScopeGuard::enter` on this thread and outlives the guard.
    let scope = unsafe { &*current };
    if scope.ctx as *const c_void != ctx {
        return ERR_NOT_ALLOWED;
    }
    let parsed = match read_command(command) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut queue = scope.queue.borrow_mut();
    if queue.len() >= MAX_SUBMITTED {
        return ERR_LIMIT;
    }
    queue.push(parsed);
    OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use poe_mod_api::{COMMAND_SEND_CHAT, ServerApi};

    fn ctx(taken: &[&str]) -> &'static ModCtx {
        Box::leak(Box::new(ModCtx {
            id: "t".into(),
            config: toml::from_str("greeting = \"hi there\"\nn = 5\nflag = true").unwrap(),
            init_phase: AtomicBool::new(true),
            commands: Mutex::default(),
            taken: Arc::new(Mutex::new(taken.iter().map(|s| (*s).to_owned()).collect())),
        }))
    }

    fn spec(name: &str, help: &str, role: u32) -> (String, String, CommandSpec) {
        let (n, h) = (name.to_owned(), help.to_owned());
        let s = CommandSpec {
            size: size_of::<CommandSpec>() as u32,
            role,
            name: Str::new(&n),
            help: Str::new(&h),
        };
        (n, h, s)
    }

    #[test]
    fn config_values_render_as_text_and_report_their_length() {
        let c = ctx(&[]);
        let p = (c as *const ModCtx).cast::<c_void>();
        let mut buf = [0u8; 16];
        let n = api_config_get(p, Str::new("greeting"), buf.as_mut_ptr(), buf.len());
        assert_eq!(&buf[..n as usize], b"hi there");
        assert_eq!(api_config_get(p, Str::new("n"), std::ptr::null_mut(), 0), 1);
        assert_eq!(
            api_config_get(p, Str::new("flag"), std::ptr::null_mut(), 0),
            4
        );
        // Cut: the full length comes back, the buffer holds the start.
        let mut small = [0u8; 2];
        assert_eq!(
            api_config_get(p, Str::new("greeting"), small.as_mut_ptr(), 2),
            8
        );
        assert_eq!(&small, b"hi");
        assert_eq!(
            api_config_get(p, Str::new("missing"), buf.as_mut_ptr(), 16),
            ERR_NOT_FOUND as isize
        );
        assert_eq!(
            api_config_get(std::ptr::null(), Str::new("n"), buf.as_mut_ptr(), 16),
            ERR_BAD_ARGUMENT as isize
        );
    }

    #[test]
    fn commands_register_only_during_init_with_valid_unique_names() {
        let c = ctx(&["help"]);
        let p = (c as *const ModCtx).cast::<c_void>();
        let (_n, _h, ok) = spec("hello", "Says hello.", ROLE_PLAYER);
        assert_eq!(api_register_command(p, &ok), OK);
        assert_eq!(api_register_command(p, &ok), ERR_EXISTS);
        let (_n, _h, taken) = spec("help", "x", ROLE_PLAYER);
        assert_eq!(api_register_command(p, &taken), ERR_EXISTS);
        let (_n, _h, bad) = spec("Bad Name", "x", ROLE_PLAYER);
        assert_eq!(api_register_command(p, &bad), ERR_BAD_ARGUMENT);
        let (_n, _h, role) = spec("rolecmd", "x", 7);
        assert_eq!(api_register_command(p, &role), ERR_BAD_ARGUMENT);
        let (_n, _h, admin) = spec("admincmd", "x", ROLE_ADMIN);
        assert_eq!(api_register_command(p, &admin), OK);
        assert_eq!(api_register_command(p, std::ptr::null()), ERR_BAD_ARGUMENT);
        let names: Vec<_> = c
            .commands
            .lock()
            .unwrap()
            .iter()
            .map(|c| (c.name.clone(), c.admin))
            .collect();
        assert_eq!(
            names,
            [("hello".to_owned(), false), ("admincmd".to_owned(), true)]
        );
        c.init_phase.store(false, Ordering::SeqCst);
        let (_n, _h, late) = spec("late", "x", ROLE_PLAYER);
        assert_eq!(api_register_command(p, &late), ERR_NOT_ALLOWED);
    }

    #[test]
    fn a_short_command_spec_is_read_by_its_own_size() {
        let c = ctx(&[]);
        let p = (c as *const ModCtx).cast::<c_void>();
        let (_n, _h, mut s) = spec("short", "x", ROLE_PLAYER);
        s.size = 4; // not even the name
        assert_eq!(api_register_command(p, &s), ERR_BAD_ARGUMENT);
    }

    #[test]
    fn submit_needs_the_scope_of_a_running_callback() {
        let c = ctx(&[]);
        let p = (c as *const ModCtx).cast::<c_void>();
        let mut cmd = Command::new(COMMAND_SEND_CHAT);
        cmd.target = 7;
        let text = "hi".to_owned();
        cmd.text = Str::new(&text);
        let scope = Scope {
            ctx: c,
            queue: RefCell::default(),
        };
        let sp = (&scope as *const Scope).cast::<c_void>();
        // Outside a callback.
        assert_eq!(api_submit(p, sp, &cmd), ERR_NOT_ALLOWED);
        {
            let _g = ScopeGuard::enter(&scope);
            assert_eq!(api_submit(p, sp, &cmd), OK);
            // A made-up scope, another mod's ctx, a bad command.
            assert_eq!(api_submit(p, std::ptr::null(), &cmd), ERR_NOT_ALLOWED);
            assert_eq!(api_submit(std::ptr::null(), sp, &cmd), ERR_NOT_ALLOWED);
            let mut bad = cmd;
            bad.kind = 99;
            assert_eq!(api_submit(p, sp, &bad), ERR_BAD_ARGUMENT);
            let mut no_target = cmd;
            no_target.target = 0;
            assert_eq!(api_submit(p, sp, &no_target), ERR_BAD_ARGUMENT);
            let mut not_utf8 = cmd;
            let junk = [0xffu8, 0xfe];
            not_utf8.text = Str {
                ptr: junk.as_ptr(),
                len: 2,
            };
            assert_eq!(api_submit(p, sp, &not_utf8), ERR_BAD_ARGUMENT);
            let mut nan = Command::new(COMMAND_TELEPORT);
            nan.target = 1;
            nan.x = f64::NAN;
            assert_eq!(api_submit(p, sp, &nan), ERR_BAD_ARGUMENT);
            assert_eq!(api_submit(p, sp, std::ptr::null()), ERR_BAD_ARGUMENT);
        }
        assert_eq!(
            api_submit(p, sp, &cmd),
            ERR_NOT_ALLOWED,
            "the scope ended with the guard"
        );
        assert_eq!(
            scope.queue.into_inner(),
            [ModCommand::SendChat {
                target: 7,
                text: "hi".into()
            }]
        );
    }

    #[test]
    fn the_queue_per_callback_is_limited() {
        let c = ctx(&[]);
        let p = (c as *const ModCtx).cast::<c_void>();
        let scope = Scope {
            ctx: c,
            queue: RefCell::default(),
        };
        let sp = (&scope as *const Scope).cast::<c_void>();
        let _g = ScopeGuard::enter(&scope);
        let mut cmd = Command::new(COMMAND_KICK);
        cmd.target = 1;
        for _ in 0..MAX_SUBMITTED {
            assert_eq!(api_submit(p, sp, &cmd), OK);
        }
        assert_eq!(api_submit(p, sp, &cmd), ERR_LIMIT);
    }

    #[test]
    fn the_abi_layout_matches_what_this_host_was_written_for() {
        // The SDK's own test pins these; repeating the sizes here fails the server build when
        // the pinned poe-mod-api (git tag) is changed to an incompatible one without a review.
        assert_eq!(poe_mod_api::SERVER_API_VERSION, 1);
        assert_eq!(size_of::<ServerApi>(), 128);
        assert_eq!(size_of::<poe_mod_api::ModInfo>(), 40);
        assert_eq!(size_of::<poe_mod_api::Event>(), 80);
        assert_eq!(size_of::<poe_mod_api::Snapshot>(), 56);
        assert_eq!(size_of::<poe_mod_api::PlayerInfo>(), 64);
        assert_eq!(size_of::<Command>(), 48);
        assert_eq!(size_of::<CommandSpec>(), 40);
    }
}
