//! The instances of areas that exist (whoever hosts them): what each one is (area, seed,
//! layout choice), who is in it, and when it went empty. Login servers pick or create the
//! instance a character enters; instance servers record players joining and leaving and close
//! instances. [`InstanceDirectory`] is a trait so a networked directory can replace
//! [`MemoryInstances`] when servers run as separate processes.

use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rand::RngCore;
use tokio::sync::watch;

/// What an instance is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstanceSpec {
    /// `WorldAreas.Id`.
    pub area: String,
    /// Terrain seed; `None`: the instance server picks one when the first player enters.
    pub seed: Option<u32>,
    /// Forced topology (`map_force_topology`: 1-based index into the area's topologies);
    /// `None`: the client's own choice.
    pub topology: Option<u32>,
    /// Forced region orientation (0..8); `None`: the client's own choice.
    pub orientation: Option<u8>,
}

/// An instance as listed.
#[derive(Debug, Clone)]
pub struct InstanceInfo {
    pub id: u32,
    pub spec: InstanceSpec,
    /// Character names of the players in it.
    pub players: Vec<String>,
    /// Unix time (ms) it was created.
    pub created_unix_ms: u64,
    /// When it went empty (or was created without players); `None` while players are in it.
    pub empty_since: Option<Instant>,
    /// Kept when empty (towns and hideouts): idle expiry skips it.
    pub persistent: bool,
}

pub trait InstanceDirectory: Send + Sync {
    /// Creates an instance (no players yet) and returns its id.
    fn create(&self, spec: InstanceSpec, persistent: bool) -> u32;
    fn get(&self, id: u32) -> Option<InstanceInfo>;
    fn list(&self) -> Vec<InstanceInfo>;
    /// The newest open instance of an area, if any.
    fn find(&self, area: &str) -> Option<u32>;
    /// Sets the seed of an instance that has none yet; returns the instance's seed.
    fn settle_seed(&self, id: u32, seed: u32) -> Option<u32>;
    fn join(&self, id: u32, player: &str);
    fn leave(&self, id: u32, player: &str);
    /// Removes an instance; its sessions see [`InstanceDirectory::closed`] change.
    fn close(&self, id: u32) -> bool;
    /// Changes when the instance is closed.
    fn closed(&self, id: u32) -> Option<watch::Receiver<bool>>;
    /// Closes instances that have been empty for `idle` (persistent ones stay); returns them.
    fn expire(&self, idle: Duration) -> Vec<u32>;
}

struct Entry {
    info: InstanceInfo,
    closed: watch::Sender<bool>,
}

/// In-process [`InstanceDirectory`].
#[derive(Default)]
pub struct MemoryInstances {
    entries: Mutex<BTreeMap<u32, Entry>>,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl InstanceDirectory for MemoryInstances {
    fn create(&self, spec: InstanceSpec, persistent: bool) -> u32 {
        let mut map = self.entries.lock().unwrap();
        let mut rng = rand::rng();
        let id = loop {
            // Shown to players as the "client-safe instance id"; kept small and readable.
            let id = rng.next_u32() % 900_000 + 100_000;
            if !map.contains_key(&id) {
                break id;
            }
        };
        let info = InstanceInfo {
            id,
            spec,
            players: Vec::new(),
            created_unix_ms: unix_ms(),
            empty_since: Some(Instant::now()),
            persistent,
        };
        map.insert(id, Entry { info, closed: watch::channel(false).0 });
        id
    }

    fn get(&self, id: u32) -> Option<InstanceInfo> {
        self.entries.lock().unwrap().get(&id).map(|e| e.info.clone())
    }

    fn list(&self) -> Vec<InstanceInfo> {
        self.entries.lock().unwrap().values().map(|e| e.info.clone()).collect()
    }

    fn find(&self, area: &str) -> Option<u32> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .filter(|e| e.info.spec.area.eq_ignore_ascii_case(area))
            .max_by_key(|e| e.info.created_unix_ms)
            .map(|e| e.info.id)
    }

    fn settle_seed(&self, id: u32, seed: u32) -> Option<u32> {
        let mut map = self.entries.lock().unwrap();
        let e = map.get_mut(&id)?;
        Some(*e.info.spec.seed.get_or_insert(seed))
    }

    fn join(&self, id: u32, player: &str) {
        if let Some(e) = self.entries.lock().unwrap().get_mut(&id) {
            e.info.players.push(player.to_owned());
            e.info.empty_since = None;
        }
    }

    fn leave(&self, id: u32, player: &str) {
        if let Some(e) = self.entries.lock().unwrap().get_mut(&id) {
            if let Some(i) = e.info.players.iter().position(|p| p == player) {
                e.info.players.remove(i);
            }
            if e.info.players.is_empty() {
                e.info.empty_since = Some(Instant::now());
            }
        }
    }

    fn close(&self, id: u32) -> bool {
        match self.entries.lock().unwrap().remove(&id) {
            Some(e) => {
                e.closed.send_replace(true);
                true
            }
            None => false,
        }
    }

    fn closed(&self, id: u32) -> Option<watch::Receiver<bool>> {
        self.entries.lock().unwrap().get(&id).map(|e| e.closed.subscribe())
    }

    fn expire(&self, idle: Duration) -> Vec<u32> {
        let old: Vec<u32> = self
            .entries
            .lock()
            .unwrap()
            .values()
            .filter(|e| !e.info.persistent && e.info.empty_since.is_some_and(|t| t.elapsed() >= idle))
            .map(|e| e.info.id)
            .collect();
        old.into_iter().filter(|&id| self.close(id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let dir = MemoryInstances::default();
        let spec = InstanceSpec { area: "G1_1".into(), ..Default::default() };
        let id = dir.create(spec, false);
        assert_eq!(dir.find("g1_1"), Some(id));
        assert_eq!(dir.settle_seed(id, 7), Some(7));
        assert_eq!(dir.settle_seed(id, 9), Some(7));
        let closed = dir.closed(id).unwrap();
        dir.join(id, "a");
        assert!(dir.expire(Duration::ZERO).is_empty());
        dir.leave(id, "a");
        assert_eq!(dir.expire(Duration::ZERO), vec![id]);
        assert!(*closed.borrow());
        assert!(dir.get(id).is_none());
        let town = dir.create(InstanceSpec { area: "G1_town".into(), ..Default::default() }, true);
        assert!(dir.expire(Duration::ZERO).is_empty());
        assert!(dir.close(town));
    }
}
