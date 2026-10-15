//! Decode any known message by its table name into a readable dump, for debug views (the
//! client DLL's Packets tab, trace tools). Messages without a Rust type return `None`.

use std::fmt::Debug;

use poe_net::Reader;

use crate::{Direction, Message, instance, login};

/// A decoded message: its `{:#?}` dump, or the decode error, and the bytes left over.
pub struct Described {
    pub text: Result<String, String>,
    pub trailing: usize,
}

fn decode<M: Message + Debug>(body: &[u8]) -> Described {
    let mut r = Reader::new(body);
    match M::decode(&mut r) {
        Ok(m) => Described {
            text: Ok(format!("{m:#?}")),
            trailing: r.remaining(),
        },
        Err(e) => Described {
            text: Err(e.to_string()),
            trailing: r.remaining(),
        },
    }
}

macro_rules! by_name {
    ($name:expr, $direction:expr, $body:expr, $($ty:path),* $(,)?) => {
        match ($name, $direction) {
            $((n, d) if n == <$ty as Message>::NAME && d == <$ty as Message>::DIRECTION => {
                Some(decode::<$ty>($body))
            })*
            _ => None,
        }
    };
}

/// Decodes `body` (the bytes after the opcode) as the message named `name` in the protocol
/// tables, sent by `direction`'s side. Login and instance names do not overlap; within a
/// channel a name can exist in both directions (`KeyExchange`).
pub fn describe(name: &str, direction: Direction, body: &[u8]) -> Option<Described> {
    by_name!(
        name,
        direction,
        body,
        login::ClientKeyExchange,
        login::ServerKeyExchange,
        login::KeepAlive,
        login::KeepAliveReply,
        login::Authenticate,
        login::AuthenticationResult,
        login::RequestCharacterList,
        login::DeleteCharacter,
        login::ResumeSession,
        login::DeleteCharacterResult,
        login::CharacterList,
        login::LeagueList,
        login::CreateCharacter,
        login::CharacterCreated,
        login::RequestCharacterDetails,
        login::CharacterDetails,
        login::PlayCharacter,
        login::InstanceDetails,
        instance::Hello,
        instance::EnableEncryption,
        instance::AreaInfo,
        instance::TerrainHashes,
        instance::LocalPlayers,
        instance::InitialState,
        instance::PlayerEntity,
        instance::SimulationFrameValue,
        instance::PingReply,
        instance::FrameReport,
        instance::ChatJoinRequest,
        instance::ChatJoined,
        instance::ExitSession,
        instance::SetPause,
        instance::PauseRequest,
        instance::LeaveGame,
        instance::ExitReport,
        instance::MoveDirection,
        instance::MoveDirectionUpdate,
        instance::UseSkill,
        instance::SkillReleased,
        instance::StartAction,
        instance::UpdateSkillTarget,
        instance::UpdateSkillTargetEntity,
        instance::UpdateActionTarget,
        instance::SetOrientation,
        instance::SprintInput,
        instance::SprintState,
        instance::StatsUpdate,
        instance::SkillStatsUpdate,
        instance::SimulationTick,
        instance::AddObject,
        instance::AddInventory,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_known_messages() {
        // PauseRequest: u8, u8 paused.
        let d = describe("PauseRequest", Direction::Client, &[0, 1]).unwrap();
        assert!(d.text.unwrap().contains("PauseRequest"));
        assert_eq!(d.trailing, 0);
        assert!(describe("PauseRequest", Direction::Server, &[0, 1]).is_none());
        assert!(describe("NoSuchMessage", Direction::Client, &[]).is_none());
    }
}
