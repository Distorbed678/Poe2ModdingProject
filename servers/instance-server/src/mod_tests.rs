//! The instance actor with mods: events with snapshots, commands applied on the actor's task,
//! and the guarantee that a mod which does nothing changes nothing on the wire.

use std::{
    ffi::c_void,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use poe_config::ModsConfig;
use poe_mod_host::{
    COMMAND_BROADCAST, COMMAND_SEND_CHAT, COMMAND_TELEPORT, Command, CommandSpec, EVENTS_ALL, Event,
    EventData, ModHost, ModInfo, ROLE_PLAYER, ServerApi, Str,
};
use poe_mod_host::{EVENT_COMMAND, EVENT_PLAYER_JOIN, EVENT_PLAYER_LEAVE, EVENT_TICK, event_bit};

use crate::{
    actor::Cmd,
    actor_tests::{Fake, Fixture},
    replay_tests::{movement_config, roll_skill},
    modlink::ModSink,
};

/// Records what the sink was asked to do.
#[derive(Default)]
struct Recorder {
    chats: Mutex<Vec<(u64, String)>>,
    broadcasts: Mutex<Vec<String>>,
}

impl ModSink for Recorder {
    fn send_chat(&self, player: u64, text: &str) -> bool {
        self.chats.lock().unwrap().push((player, text.to_owned()));
        true
    }
    fn broadcast(&self, text: &str) {
        self.broadcasts.lock().unwrap().push(text.to_owned());
    }
    fn kick(&self, _: u64, _: &str) -> bool {
        true
    }
}

fn fixture_with(mods: ModHost, sink: Arc<dyn ModSink>) -> Fixture {
    let mods = Arc::new(mods);
    Fixture::customised(12000, move |mut init| {
        init.mods = mods;
        init.sink = sink;
        init
    })
}

fn cfg() -> ModsConfig {
    ModsConfig::default()
}

static JOINS: AtomicUsize = AtomicUsize::new(0);
static LEAVES: AtomicUsize = AtomicUsize::new(0);
static TICKS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn teleporter_event(user: *mut c_void, event: *const Event) -> i32 {
    let api = unsafe { &*(user as *const ServerApi) };
    let e = unsafe { &*event };
    match e.kind {
        EVENT_PLAYER_JOIN => {
            JOINS.fetch_add(1, Ordering::SeqCst);
            let who = unsafe { &*e.player };
            let snap = unsafe { &*e.snapshot };
            assert!(unsafe { snap.players() }.iter().any(|p| p.id == who.id), "the newcomer is in the join snapshot");
            // Move the newcomer to (40, 50) and greet everyone.
            let mut tp = Command::new(COMMAND_TELEPORT);
            tp.target = who.id;
            tp.x = 40.0;
            tp.y = 50.0;
            assert_eq!((api.submit)(api.ctx, e.scope, &tp), 0);
            let text = format!("{} joined", unsafe { who.character.as_str() });
            let mut b = Command::new(COMMAND_BROADCAST);
            b.text = Str::new(&text);
            assert_eq!((api.submit)(api.ctx, e.scope, &b), 0);
        }
        EVENT_PLAYER_LEAVE => {
            LEAVES.fetch_add(1, Ordering::SeqCst);
            let snap = unsafe { &*e.snapshot };
            assert!(unsafe { snap.players() }.iter().all(|p| p.id != unsafe { (*e.player).id }), "the leaver is no longer in the snapshot");
        }
        EVENT_TICK => {
            TICKS.fetch_add(1, Ordering::SeqCst);
        }
        EVENT_COMMAND => {
            let who = unsafe { &*e.player };
            let mut c = Command::new(COMMAND_SEND_CHAT);
            c.target = who.id;
            let text = format!("pos {:.0} {:.0}", who.x, who.y);
            c.text = Str::new(&text);
            assert_eq!((api.submit)(api.ctx, e.scope, &c), 0);
        }
        _ => {}
    }
    0
}

extern "C" fn teleporter_init(api: *const ServerApi, info: *mut ModInfo) -> i32 {
    let a = unsafe { &*api };
    let spec = CommandSpec { size: size_of::<CommandSpec>() as u32, role: ROLE_PLAYER, name: Str::new("pos"), help: Str::new("Your cell.") };
    assert_eq!((a.register_command)(a.ctx, &spec), 0);
    let i = unsafe { &mut *info };
    i.user = api as *mut c_void;
    i.event_mask = event_bit(EVENT_PLAYER_JOIN) | event_bit(EVENT_PLAYER_LEAVE) | event_bit(EVENT_TICK);
    i.on_event = Some(teleporter_event);
    0
}

#[test]
fn a_mod_teleports_a_joining_player_and_talks_through_the_sink() {
    let sink = Arc::new(Recorder::default());
    let mods = ModHost::from_inits(&[("teleporter", teleporter_init)], cfg());
    let mut fx = fixture_with(mods, sink.clone());
    let alice = fx.join("Alice", (10, 10), movement_config(&[roll_skill(None)]));
    // The teleport was queued by the actor inside the join, after the callback returned; like
    // the admin's `teleport` command the world carries it out in the next frame.
    fx.tick();
    assert_eq!(fx.cell(&alice), (40, 50));
    assert_eq!(sink.broadcasts.lock().unwrap().as_slice(), ["Alice joined"]);
    assert!(JOINS.load(Ordering::SeqCst) >= 1);

    let ticks = TICKS.load(Ordering::SeqCst);
    fx.tick();
    assert!(TICKS.load(Ordering::SeqCst) > ticks);

    // The mod's chat command: delivered with the caller and a full snapshot, answered through
    // the sink with the caller's own position.
    fx.actor.handle(Cmd::ModCommand { session: alice.session, mod_id: "teleporter".into(), name: "pos".into(), args: String::new() });
    assert_eq!(sink.chats.lock().unwrap().as_slice(), [(alice.session, "pos 40 50".to_owned())]);

    let left = LEAVES.load(Ordering::SeqCst);
    fx.leave(&alice);
    assert!(LEAVES.load(Ordering::SeqCst) > left);
}

#[test]
fn a_command_for_a_player_who_left_is_dropped() {
    let sink = Arc::new(Recorder::default());
    let mods = ModHost::from_inits(&[("teleporter", teleporter_init)], cfg());
    let mut fx = fixture_with(mods, sink.clone());
    let alice = fx.join("Alice", (10, 10), movement_config(&[roll_skill(None)]));
    fx.leave(&alice);
    fx.actor.handle(Cmd::ModCommand { session: alice.session, mod_id: "teleporter".into(), name: "pos".into(), args: String::new() });
    assert!(sink.chats.lock().unwrap().is_empty());
}

extern "C" fn noop_event(_: *mut c_void, _: *const Event) -> i32 {
    0
}

extern "C" fn noop_init(_: *const ServerApi, info: *mut ModInfo) -> i32 {
    let i = unsafe { &mut *info };
    i.event_mask = EVENTS_ALL;
    i.on_event = Some(noop_event);
    0
}

/// Everything the actor sends for a fixed script: joins, walking, a teleport, a leave.
fn script(mut fx: Fixture) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut a = fx.join("Alice", (10, 10), movement_config(&[roll_skill(None)]));
    let mut out = Vec::new();
    let grab = |a: &mut Fake, out: &mut Vec<(Vec<u8>, Vec<u8>)>| {
        while let Ok(f) = a.rx.try_recv() {
            out.push((f.head.clone(), f.events.as_ref().clone()));
        }
    };
    fx.walk(&a, 1, 0);
    for _ in 0..20 {
        fx.tick();
    }
    grab(&mut a, &mut out);
    let mut b = fx.join("Bob", (12, 10), movement_config(&[roll_skill(None)]));
    for _ in 0..10 {
        fx.tick();
    }
    fx.teleport(&a, (20, 20));
    for _ in 0..5 {
        fx.tick();
    }
    grab(&mut a, &mut out);
    grab(&mut b, &mut out);
    fx.leave(&b);
    fx.tick();
    grab(&mut a, &mut out);
    out
}

#[test]
fn a_mod_that_does_nothing_changes_nothing_on_the_wire() {
    let plain = script(Fixture::new());
    assert!(!plain.is_empty());
    let mods = ModHost::from_inits(&[("noop", noop_init)], cfg());
    let with_mod = script(fixture_with(mods, Arc::new(crate::modlink::NullSink)));
    assert_eq!(plain, with_mod, "frames must be byte-identical with a no-op mod subscribed to every event");
}

#[test]
fn command_events_reach_only_the_owning_mod() {
    let mods = ModHost::from_inits(&[("noop", noop_init)], cfg());
    let ev = EventData::server(EVENT_COMMAND);
    assert!(mods.dispatch_command("somebody-else", &ev).is_empty());
}
