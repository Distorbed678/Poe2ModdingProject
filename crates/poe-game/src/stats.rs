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
}

impl StatRegistry {
    pub fn load(data: &GameData) -> Result<Self> {
        let table = data.table("Stats")?;
        let mut keys = HashMap::with_capacity(table.len());
        for i in 0..table.len() {
            if let Some(row) = table.row(i) {
                keys.insert(row.str("Id")?, i as u32 + 1);
            }
        }
        Ok(Self { keys })
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
