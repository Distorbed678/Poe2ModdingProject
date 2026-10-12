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
}

impl StartState {
    /// Looks up a start state by its id (e.g. `"DexFourDefault"`).
    pub fn find(data: &GameData, id: &str) -> Result<Option<Self>> {
        let states = data.table("CharacterStartStates")?;
        let Some(row) = states.find_by_str("Id", id)? else {
            return Ok(None);
        };
        let level = row.int("Level")? as u32;
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
        }))
    }
}
