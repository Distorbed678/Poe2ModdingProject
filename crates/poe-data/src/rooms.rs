//! Terrain rooms (`.arm`): the pre-authored pieces the terrain generator places.
//!
//! A room is a grid of slots (`side_length` tiles each) with tagged slots (e.g.
//! `intro_feature`, filled with a feature tile such as `Intro_Tree`) and points of interest:
//! named positions in grid cells (23 per tile) relative to the room's corner, e.g. the
//! `default1`/`default2` entry points of an area's entrance room. Only what the server needs
//! to locate those points is kept; parsing is done by `poe_data_tools`.

use poe_data_tools::file_parsers::{
    FileParser,
    arm::{ARMParser, types::Slot},
};

use crate::{DataError, GameData, Result};

/// A tagged slot, in tiles relative to the room's corner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomSlot {
    pub tag: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A point of interest, in grid cells relative to the room's corner.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomPoint {
    pub tag: String,
    pub x: u32,
    pub y: u32,
    pub rotation: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Room {
    /// Room name (`entrance`, `tutorial1_river`, ...).
    pub name: String,
    /// Size in tiles.
    pub width: u32,
    pub height: u32,
    pub slots: Vec<RoomSlot>,
    pub points: Vec<RoomPoint>,
}

impl Room {
    /// Loads a room by path, e.g. `Metadata/Terrain/Gallows/Act1/1_1/Rooms/entrance_intro.arm`.
    pub fn load(data: &GameData, path: &str) -> Result<Self> {
        let bytes = data.files().read(&path.to_lowercase())?;
        let arm = ARMParser.parse(&bytes).map_err(|e| DataError::Malformed {
            table: path.to_owned(),
            message: format!("{e:?}"),
        })?;
        let unit = arm.dimensions.side_length.max(1);
        let (width, height) = match &arm.root_slot {
            Slot::K(k) => (k.width * unit, k.height * unit),
            _ => {
                return Err(DataError::Malformed {
                    table: path.to_owned(),
                    message: "root slot is not a k slot".into(),
                });
            }
        };
        let mut slots = Vec::new();
        for (gy, row) in arm.grid.iter().enumerate() {
            for (gx, slot) in row.iter().enumerate() {
                if let Slot::K(k) = slot
                    && let Some(tag) = &k.slot_tag
                {
                    slots.push(RoomSlot {
                        tag: tag.clone(),
                        x: gx as u32 * unit,
                        y: gy as u32 * unit,
                        width: k.width * unit,
                        height: k.height * unit,
                    });
                }
            }
        }
        let points = arm
            .points_of_interest
            .iter()
            .flatten()
            .map(|p| RoomPoint {
                tag: p.tag.clone(),
                x: p.x,
                y: p.y,
                rotation: p.rotation,
            })
            .collect();
        Ok(Self {
            name: arm.tag,
            width,
            height,
            slots,
            points,
        })
    }

    pub fn slot(&self, tag: &str) -> Option<&RoomSlot> {
        self.slots.iter().find(|s| s.tag == tag)
    }

    pub fn point(&self, tag: &str) -> Option<&RoomPoint> {
        self.points.iter().find(|p| p.tag == tag)
    }
}
