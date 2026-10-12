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
    /// 16-bit area hash the client uses to reference areas compactly.
    pub hash16: u16,
}

impl WorldArea {
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
            hash16: row.int("HASH16")? as u16,
        }))
    }
}
