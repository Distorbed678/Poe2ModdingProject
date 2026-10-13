//! Typed views over the tables the servers use. Column names follow the community schema.

use crate::{GameData, Result};

/// A `CharacterStartStates` row resolved to its class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartState {
    pub id: String,
    /// `Characters.IntegerId` of the class.
    pub class_id: u32,
    /// `Characters.Name`, e.g. "Ranger".
    pub class_name: String,
    pub level: u32,
    /// `WorldAreas.Id` of the start state's hub area, if it has one.
    pub hub_area: Option<String>,
}

impl StartState {
    /// Looks up a start state by its id (e.g. `"DexFourDefault"`).
    pub fn find(data: &GameData, id: &str) -> Result<Option<Self>> {
        let states = data.table("CharacterStartStates")?;
        let Some(row) = states.find_by_str("Id", id)? else {
            return Ok(None);
        };
        let level = row.int("Level")? as u32;
        let hub_area = match row.row_ref("HubArea")? {
            Some(i) => data
                .table("WorldAreas")?
                .row(i)
                .map(|r| r.str("Id"))
                .transpose()?,
            None => None,
        };
        let Some(character) = row.row_ref("Character")? else {
            return Ok(None);
        };
        let characters = data.table("Characters")?;
        let Some(class) = characters.row(character) else {
            return Ok(None);
        };
        Ok(Some(Self {
            id: id.to_owned(),
            class_id: class.int("IntegerId")? as u32,
            class_name: class.str("Name")?,
            level,
            hub_area,
        }))
    }
}

/// A `WorldAreas` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldArea {
    pub id: String,
    pub name: String,
    pub act: i64,
    pub level: i64,
    pub is_town: bool,
    pub is_hideout: bool,
    /// 16-bit area hash the client uses to reference areas compactly.
    pub hash16: u16,
}

impl WorldArea {
    /// Towns and hideouts always use layout seed 1 (every one of them in live client logs);
    /// other areas get a random seed per instance.
    pub fn fixed_seed(&self) -> Option<u32> {
        (self.is_town || self.is_hideout).then_some(1)
    }

    pub fn find(data: &GameData, id: &str) -> Result<Option<Self>> {
        let areas = data.table("WorldAreas")?;
        let Some(row) = areas.find_by_str("Id", id)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            id: id.to_owned(),
            name: row.str("Name")?,
            act: row.int("Act")?,
            level: row.int("AreaLevel")?,
            is_town: row.bool("IsTown")?,
            is_hideout: row.bool("IsHideout")?,
            hash16: row.int("HASH16")? as u16,
        }))
    }
}

/// A `Characters` row (a playable class).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterClass {
    /// `Characters.AOFile`: the class's animated object (model).
    pub ao_file: String,
    pub base_strength: i64,
    pub base_dexterity: i64,
    pub base_intelligence: i64,
    /// Metadata path of the class's object type, e.g. `Metadata/Characters/Dex/DexFour`.
    pub metadata_id: String,
    pub name: String,
    pub integer_id: u32,
    pub base_max_life: i64,
    pub base_max_mana: i64,
}

impl CharacterClass {
    /// Looks up a class by `Characters.IntegerId` (the id stored with each character).
    pub fn by_integer_id(data: &GameData, integer_id: u32) -> Result<Option<Self>> {
        let characters = data.table("Characters")?;
        for i in 0..characters.len() {
            let Some(row) = characters.row(i) else {
                continue;
            };
            if row.int("IntegerId")? as u32 == integer_id {
                return Ok(Some(Self {
                    metadata_id: row.str("Id")?,
                    name: row.str("Name")?,
                    integer_id,
                    base_max_life: row.int("BaseMaxLife")?,
                    base_max_mana: row.int("BaseMaxMana")?,
                    ao_file: row.str("AOFile")?,
                    base_strength: row.int("BaseStrength")?,
                    base_dexterity: row.int("BaseDexterity")?,
                    base_intelligence: row.int("BaseIntelligence")?,
                }));
            }
        }
        Ok(None)
    }

    /// Object type key the client uses for this class's player entity.
    pub fn object_type_key(&self) -> u32 {
        crate::hash::object_type_key(&self.metadata_id)
    }
}

/// An `Inventories` row: a kind of inventory (main bag, an equipment slot, ...). The row index
/// is the slot index the client uses (inventory +0x144; 121 rows in 0.5.5e).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySlot {
    pub index: u8,
    pub id: String,
    pub id_key: i64,
}

impl InventorySlot {
    pub fn all(data: &GameData) -> Result<Vec<Self>> {
        let table = data.table("Inventories")?;
        (0..table.len())
            .filter_map(|i| table.row(i).map(|r| (i, r)))
            .map(|(i, row)| {
                Ok(Self {
                    index: i as u8,
                    id: row.str("Id")?,
                    id_key: row.int("InventoryIdKey")?,
                })
            })
            .collect()
    }
}

/// Index of a `LeagueFlag` row by id. The client identifies a character's league flag
/// (banner and icon) by row + 1; 0 or 1 select the generic event banner.
pub fn league_flag_index(data: &GameData, id: &str) -> Result<Option<u32>> {
    let table = data.table("LeagueFlag")?;
    for i in 0..table.len() {
        if let Some(row) = table.row(i)
            && row.str("Id")? == id
        {
            return Ok(Some(i as u32));
        }
    }
    Ok(None)
}

/// Network key of a stat: `Stats.dat` row + 1 (the client queries e.g. `actor_scale_+%`,
/// row 1913, as 1914).
pub fn stat_key(data: &GameData, id: &str) -> Result<Option<u32>> {
    let table = data.table("Stats")?;
    Ok(table.find_index_by_str("Id", id)?.map(|i| i as u32 + 1))
}
