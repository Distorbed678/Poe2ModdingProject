//! Character stats.
//!
//! The client keys stats by `Stats.dat` row + 1 ([`StatRegistry`]). A freshly created
//! character's stats come from:
//! - the object template's `Stats` section (`Metadata/Characters/Character.ot`: `level`,
//!   `is_player`, `life_per_level`, ...);
//! - the class row in `Characters.dat` (base life/mana and attributes);
//! - the character's level.
//!
//! Derived totals use the template's per-level stats:
//! `maximum_life = base_maximum_life + life_per_level * level` (and the same for mana).
//! Attribute bonuses and modifiers (passives, items, buffs) are not applied yet.

use std::collections::{BTreeMap, HashMap};

use poe_data::{GameData, Result, template::ObjectTemplate, views::CharacterClass};

/// Stat id (`Stats.Id`) -> network key, loaded once.
pub struct StatRegistry {
    keys: HashMap<String, u32>,
    /// Stats the server sends to the other players too (`Stats.IsParty`); the rest only go to
    /// the entity's own client (official captures: `stun_threshold_+%_final_while_performing_action`
    /// is in a local roll batch but never in a remote one).
    party: std::collections::HashSet<u32>,
}

impl StatRegistry {
    pub fn load(data: &GameData) -> Result<Self> {
        let table = data.table("Stats")?;
        let mut keys = HashMap::with_capacity(table.len());
        let mut party = std::collections::HashSet::new();
        for i in 0..table.len() {
            if let Some(row) = table.row(i) {
                keys.insert(row.str("Id")?, i as u32 + 1);
                if row.bool("IsParty")? {
                    party.insert(i as u32 + 1);
                }
            }
        }
        Ok(Self { keys, party })
    }

    /// A registry from explicit rows `(stat id, network key, IsParty)`, for tests and tools that
    /// have no game files.
    pub fn from_rows(rows: &[(&str, u32, bool)]) -> Self {
        Self {
            keys: rows
                .iter()
                .map(|&(id, key, _)| (id.to_owned(), key))
                .collect(),
            party: rows.iter().filter(|r| r.2).map(|r| r.1).collect(),
        }
    }

    /// Whether other players' clients are told about this stat (`Stats.IsParty`).
    pub fn is_party(&self, id: &str) -> bool {
        self.key(id).is_some_and(|k| self.party.contains(&k))
    }

    pub fn key(&self, id: &str) -> Option<u32> {
        self.keys.get(id).copied()
    }
}

/// Stat values by id, in a stable order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats(pub BTreeMap<String, i32>);

impl Stats {
    pub fn get(&self, id: &str) -> i32 {
        self.0.get(id).copied().unwrap_or(0)
    }

    pub fn set(&mut self, id: &str, value: i32) {
        self.0.insert(id.to_owned(), value);
    }

    /// (network key, value) pairs; stats unknown to `registry` or zero are skipped.
    pub fn to_network(&self, registry: &StatRegistry) -> Vec<(u32, i32)> {
        self.0
            .iter()
            .filter(|(_, v)| **v != 0)
            .filter_map(|(id, v)| {
                let key = registry.key(id);
                if key.is_none() {
                    tracing::debug!(stat = %id, "stat not in Stats.dat");
                }
                Some((key?, *v))
            })
            .collect()
    }
}

/// The engine's default energy shield recharge delay in milliseconds (the stat
/// `virtual_total_energy_shield_recharge_delay_ms` of every official character without delay
/// modifiers; 2000 with `energy_shield_delay_-%` 100). Not in any game table.
pub const ES_RECHARGE_DELAY_MS: i32 = 4000;

/// Adds the derived stats other players' clients get for a character that stands at full life,
/// mana and energy shield with nothing equipped (the capture of an official town lists them for
/// every player): the "on full ..." conditions, `poise_threshold` and
/// `intermediary_poise_threshold` (= maximum life) and the energy shield recharge delay.
/// Everything else the official server derives (armour, evasion, accuracy, regeneration, ...)
/// needs a stat calculator and is left out.
pub fn add_resting_state(stats: &mut Stats) {
    for id in [
        "virtual_on_full_life",
        "on_full_mana",
        "on_full_energy_shield",
        "above_90%_mana",
    ] {
        stats.set(id, 1);
    }
    let life = stats.get("maximum_life");
    stats.set("poise_threshold", life);
    stats.set("intermediary_poise_threshold", life);
    let delay_less = stats.get("energy_shield_delay_-%").max(0) as i64;
    let delay = i64::from(ES_RECHARGE_DELAY_MS) * 100 / (100 + delay_less);
    stats.set(
        "virtual_total_energy_shield_recharge_delay_ms",
        delay as i32,
    );
}

/// Default stats of a new character of `class` at `level`.
pub fn character_defaults(template: &ObjectTemplate, class: &CharacterClass, level: u32) -> Stats {
    let mut stats = Stats::default();
    for (id, value) in template.component("Stats") {
        let v = match value.as_str() {
            "true" => 1,
            "false" => 0,
            v => match v.parse::<i64>() {
                Ok(n) => n.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                // Non-integer template values (none in Character.ot today) are ignored.
                Err(_) => continue,
            },
        };
        stats.set(id, v);
    }
    let level = level.max(1) as i32;
    stats.set("level", level);
    stats.set("base_maximum_life", class.base_max_life as i32);
    stats.set("base_maximum_mana", class.base_max_mana as i32);
    stats.set("base_strength", class.base_strength as i32);
    stats.set("base_dexterity", class.base_dexterity as i32);
    stats.set("base_intelligence", class.base_intelligence as i32);

    let per_level = |id: &str| stats.get(id) * level;
    let strength = stats.get("base_strength") + stats.get("strength_per_level") * (level - 1);
    let dexterity = stats.get("base_dexterity") + stats.get("dexterity_per_level") * (level - 1);
    let intelligence =
        stats.get("base_intelligence") + stats.get("intelligence_per_level") * (level - 1);
    let maximum_life = stats.get("base_maximum_life") + per_level("life_per_level");
    let maximum_mana = stats.get("base_maximum_mana") + per_level("mana_per_level");
    stats.set("strength", strength);
    stats.set("dexterity", dexterity);
    stats.set("intelligence", intelligence);
    stats.set("maximum_life", maximum_life);
    stats.set("maximum_mana", maximum_mana);
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resting_state_adds_the_town_conditions() {
        let mut stats = Stats::default();
        stats.set("maximum_life", 188);
        add_resting_state(&mut stats);
        for id in [
            "virtual_on_full_life",
            "on_full_mana",
            "on_full_energy_shield",
        ] {
            assert_eq!(stats.get(id), 1, "{id}");
        }
        assert_eq!(stats.get("poise_threshold"), 188);
        assert_eq!(stats.get("intermediary_poise_threshold"), 188);
        // 4000 ms; 2000 with `energy_shield_delay_-%` 100 (the official Witch).
        let delay = "virtual_total_energy_shield_recharge_delay_ms";
        assert_eq!(stats.get(delay), 4000);
        stats.set("energy_shield_delay_-%", 100);
        add_resting_state(&mut stats);
        assert_eq!(stats.get(delay), 2000);
    }

    #[test]
    fn registry_rows_mark_public_stats() {
        let registry = StatRegistry::from_rows(&[("a", 1, true), ("b", 2, false)]);
        assert!(registry.is_party("a"));
        assert!(!registry.is_party("b"));
        assert!(!registry.is_party("missing"));
        assert_eq!(registry.key("b"), Some(2));
    }

    #[test]
    fn ranger_level_one() {
        let mut template = ObjectTemplate::default();
        template.components.insert(
            "Stats".into(),
            vec![
                ("level".into(), "1".into()),
                ("is_player".into(), "1".into()),
                ("life_per_level".into(), "12".into()),
                ("mana_per_level".into(), "4".into()),
                ("enable_weapon_sets".into(), "true".into()),
            ],
        );
        let class = CharacterClass {
            metadata_id: "Metadata/Characters/Dex/DexFour".into(),
            name: "Ranger".into(),
            integer_id: 2,
            base_max_life: 16,
            base_max_mana: 30,
            ao_file: "Metadata/Characters/Dex/DexFour.ao".into(),
            base_strength: 7,
            base_dexterity: 15,
            base_intelligence: 7,
        };
        let s = character_defaults(&template, &class, 1);
        assert_eq!(s.get("maximum_life"), 28);
        assert_eq!(s.get("maximum_mana"), 34);
        assert_eq!(s.get("dexterity"), 15);
        assert_eq!(s.get("is_player"), 1);
        assert_eq!(s.get("enable_weapon_sets"), 1);
    }
}
