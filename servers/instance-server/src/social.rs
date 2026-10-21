//! Party invitations and trade requests between the sessions in game (WP C3,
//! docs/protocol/entities.md#party-and-trade-flows-wp-c3).
//!
//! These are account-session messages, not simulation messages: the session writes them to
//! its client as soon as they arrive (like chat lines), outside the instance actor's frames.
//! A [`Social`] holds the open parties and trades and one queue of [`SocialOut`] per session.
//!
//! Implemented from what the official capture shows: invite, cancel (party), request and
//! cancel (trade). **Not implemented (gate G-PARTY):** accepting and declining a party
//! invitation or a trade, timeouts, members that joined, leaving a party. What the invitee's
//! client receives was never captured either: it is derived from the client's handlers
//! (`0x142106a30` case `0xc8`, `0x1402c3cb0`) and verified in game with two clients (WP C3, 2026-10-21): the popups appear.

use std::{collections::HashMap, sync::Mutex};

use poe_proto::{
    instance::{AddInventory, Inventory},
    opcodes::OpcodeTable,
    remote::{
        NoticeMessage, PartyEnded, PartyMember, PartyUpdate, SocialEntry, SocialRemove, TradeAck,
        TradeEvent, TradeState,
    },
};
use tokio::sync::mpsc;

/// Party roles in a [`PartyMember`].
const ROLE_INVITED: u8 = 0;
const ROLE_LEADER: u8 = 2;
/// `PartyEnded.reason` after the invitation was cancelled (capture).
const ENDED_CANCELLED: u8 = 4;
/// `SocialEntry` flags: bit 2 character data, bit 0 two more `u16`s (capture: 5 for others and
/// for yourself in a town, 4 for yourself in an instance).
const ENTRY_OTHER: u8 = 5;
const ENTRY_SELF_IN_INSTANCE: u8 = 4;
/// `SocialEntry.unk_1fc`: `0x9b` in every captured entry.
const ENTRY_UNK_1FC: u8 = 0x9b;
/// `TradeState` constants of the capture (`unk_a`, `unk_b`, `unk_d`, `unk_e`).
const TRADE_UNK_A: u32 = 2;
const TRADE_UNK_B: u32 = 1;
const TRADE_NO_NPC: u32 = 0xffff_ffff;
const TRADE_UNK_E: u16 = 0x6a;
/// The two trade-window inventories an official server creates (`AddInventory`) right before
/// the `TradeState` (capture 2026-10-20: ids 1 and 2, `InventoryIdKey` `0x49`, `Inventories`
/// slot `0x79`, 12x5, flags `0x40`). Without them the client throws "Invalid inventory id"
/// while it builds the trade window and crashes (found in the WP C3 in-game check).
const TRADE_INVENTORY_IDS: [u32; 2] = [1, 2];
const TRADE_INVENTORY: Inventory = Inventory {
    id_key: 0x49,
    slot: 0x79,
    width: 12,
    height: 5,
    flags: 0x40,
};
/// `TradeState.unk_c` for the player who asked (capture: `0x31`) and, verified in game, for the
/// target: bit 1 set and bit 0 clear is what the client's invite popup tests (`0x1402c3cb0`).
const TRADE_FLAGS_REQUESTER: u8 = 0x31;
const TRADE_FLAGS_TARGET: u8 = 0x32;
/// `TradeEvent` kinds and values sent after a cancel (capture: `(1, 2)` then `(1, 1)`).
const TRADE_EVENT_KIND: u8 = 1;
const TRADE_EVENT_VALUES: [u32; 2] = [2, 1];
/// `NoticeMessage` id for "trade cancelled" (capture) and its constant fields.
const NOTICE_TRADE_CANCELLED: u16 = 0xe1;
const NOTICE_UNK_A: u16 = 0xffff;
const NOTICE_UNK_B: u16 = 0x0200;

/// What the client needs to know about a character in game.
#[derive(Debug, Clone)]
pub struct Person {
    pub character: String,
    pub account: String,
    pub level: u8,
    /// `Characters.IntegerId`.
    pub class: u8,
    /// `WorldAreas.HASH16` of the current area.
    pub area: u16,
    pub is_town: bool,
    pub league: String,
    /// The instance and the entity id of the character in it (trade targets are entities).
    pub instance: Option<u32>,
    pub entity: u32,
    /// The client's local player index (`u8`, 1).
    pub local_index: u8,
    /// The connection ([`crate::actor::SessionId`]): a newer session of the same character
    /// replaces this one.
    pub session: u64,
}

/// A message for a session to write to its client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocialOut {
    AddInventory(AddInventory),
    Entry(SocialEntry),
    Removed(SocialRemove),
    PartyUpdate(PartyUpdate),
    PartyEnded(PartyEnded),
    TradeState(TradeState),
    TradeAck(TradeAck),
    TradeEvent(TradeEvent),
    Notice(NoticeMessage),
}

impl SocialOut {
    /// The packet: opcode and body.
    pub fn encode(&self, table: &OpcodeTable) -> Result<Vec<u8>, poe_proto::ProtocolError> {
        match self {
            Self::AddInventory(m) => table.encode(m),
            Self::Entry(m) => table.encode(m),
            Self::Removed(m) => table.encode(m),
            Self::PartyUpdate(m) => table.encode(m),
            Self::PartyEnded(m) => table.encode(m),
            Self::TradeState(m) => table.encode(m),
            Self::TradeAck(m) => table.encode(m),
            Self::TradeEvent(m) => table.encode(m),
            Self::Notice(m) => table.encode(m),
        }
    }
}

struct Member {
    person: Person,
    tx: mpsc::UnboundedSender<SocialOut>,
}

/// A party that only has its leader and the characters invited so far.
struct Party {
    id: u32,
    /// Lower-case character names.
    leader: String,
    invited: Vec<String>,
}

struct Trade {
    id: u32,
    requester: String,
    target: String,
}

struct State {
    people: HashMap<String, Member>,
    parties: Vec<Party>,
    trades: Vec<Trade>,
    next_party: u32,
    next_trade: u32,
}

/// The open parties and trades and the queues of the sessions in game.
pub struct Social {
    state: Mutex<State>,
}

impl Default for Social {
    fn default() -> Self {
        Self::starting_at(1, 1)
    }
}

impl Social {
    /// Party and trade ids count up from these (the capture's first ids were `0xcfc43` and 1).
    pub fn starting_at(first_party: u32, first_trade: u32) -> Self {
        Self {
            state: Mutex::new(State {
                people: HashMap::new(),
                parties: Vec::new(),
                trades: Vec::new(),
                next_party: first_party,
                next_trade: first_trade,
            }),
        }
    }

    /// A session entered the game; the returned queue holds what to write to its client.
    pub fn register(&self, person: Person) -> mpsc::UnboundedReceiver<SocialOut> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut st = self.state.lock().unwrap();
        let key = person.character.to_lowercase();
        // A newer session of the same character replaces the old one: its parties and trades
        // end as if the character had left.
        if st.people.contains_key(&key) {
            st.drop_person(&key);
        }
        st.people.insert(key, Member { person, tx });
        rx
    }

    /// The session ended (only if it is still `session`): its open parties and trades end.
    pub fn unregister(&self, character: &str, session: u64) {
        let mut st = self.state.lock().unwrap();
        let key = character.to_lowercase();
        if st
            .people
            .get(&key)
            .is_some_and(|m| m.person.session == session)
        {
            st.drop_person(&key);
        }
    }

    /// `PartyInvite`: `from` invites the character `to`. The error is a line for `from`'s chat.
    pub fn invite(&self, from: &str, to: &str) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let (a, b) = (from.to_lowercase(), to.trim().to_lowercase());
        if !st.people.contains_key(&a) {
            return Err("You are not in game.".into());
        }
        if a == b {
            return Err("You cannot invite yourself.".into());
        }
        let Some(target) = st.people.get(&b) else {
            return Err(format!("{} is not online.", to.trim()));
        };
        let target_name = target.person.character.clone();
        if st
            .party_of_leader(&a)
            .is_some_and(|p| p.invited.contains(&b))
        {
            return Err(format!("{target_name} is already invited."));
        }
        if st
            .parties
            .iter()
            .any(|p| p.leader == b || p.invited.contains(&b))
        {
            return Err(format!(
                "{target_name} is already in a party or invited to one."
            ));
        }
        if st.parties.iter().any(|p| p.invited.contains(&a)) {
            return Err("Answer the invitation you have first.".into());
        }
        // A new invitation starts a new party (a new id), unless the leader has one open.
        let new_party = st.party_of_leader(&a).is_none();
        if new_party {
            let id = st.next_party;
            st.next_party += 1;
            st.parties.push(Party {
                id,
                leader: a.clone(),
                invited: Vec::new(),
            });
        }
        let party = st.parties.iter_mut().find(|p| p.leader == a).unwrap();
        party.invited.push(b.clone());
        let (id, invited) = (party.id, party.invited.clone());

        // The inviter (capture): its own entry and the one-member party twice, then the
        // invitee's entry and the party with the invitee.
        let leader = st.people[&a].person.clone();
        let invitee = st.people[&b].person.clone();
        let mut to_leader = Vec::new();
        if new_party {
            let alone = st.party_update(id, &leader, &[], &leader, 0);
            to_leader.push(SocialOut::Entry(entry(&leader, true)));
            to_leader.push(SocialOut::PartyUpdate(alone.clone()));
            to_leader.push(SocialOut::PartyUpdate(alone));
        }
        to_leader.push(SocialOut::Entry(entry(&invitee, false)));
        let everyone: Vec<Person> = invited
            .iter()
            .map(|n| st.people[n].person.clone())
            .collect();
        to_leader.push(SocialOut::PartyUpdate(
            st.party_update(id, &leader, &everyone, &leader, 0),
        ));
        st.send(&a, to_leader);
        // The invitee (never captured, verified in game): the leader's entry, so the client can name
        // the leader, then the party with itself as an invited member.
        st.send(
            &b,
            vec![
                SocialOut::Entry(entry(&leader, false)),
                SocialOut::PartyUpdate(st.party_update(id, &leader, &everyone, &invitee, 0)),
            ],
        );
        Ok(())
    }

    /// `PartyInviteCancel`: `from` takes back the invitation of `to`.
    pub fn cancel_invite(&self, from: &str, to: &str) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let (a, b) = (from.to_lowercase(), to.trim().to_lowercase());
        let Some(party) = st.party_of_leader(&a) else {
            return Err("You have no open invitation.".into());
        };
        if !party.invited.contains(&b) {
            return Err(format!("{} is not invited.", to.trim()));
        }
        st.end_invitation(&a, &b);
        Ok(())
    }

    /// `TradeRequest`: `from` asks the player behind `target` (an entity id in `from`'s
    /// instance) to trade. The error is a line for `from`'s chat.
    pub fn trade_request(&self, from: &str, target: u32) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let a = from.to_lowercase();
        let Some(requester) = st.people.get(&a).map(|m| m.person.clone()) else {
            return Err("You are not in game.".into());
        };
        let Some(other) = st
            .people
            .values()
            .map(|m| &m.person)
            .find(|p| p.instance == requester.instance && p.entity == target)
            .cloned()
        else {
            return Err("That player is not here.".into());
        };
        let b = other.character.to_lowercase();
        if a == b {
            return Err("You cannot trade with yourself.".into());
        }
        if st.trades.iter().any(|t| {
            [&t.requester, &t.target]
                .iter()
                .any(|n| **n == a || **n == b)
        }) {
            return Err("Either of you is already trading.".into());
        }
        let id = st.next_trade;
        st.next_trade += 1;
        st.trades.push(Trade {
            id,
            requester: a.clone(),
            target: b.clone(),
        });
        let state = |viewer: &Person, with: &Person, flags: u8| {
            SocialOut::TradeState(TradeState {
                local_index: viewer.local_index,
                trade: id,
                unk_a: TRADE_UNK_A,
                unk_b: TRADE_UNK_B,
                unk_c: flags,
                other: with.character.clone(),
                unk_d: TRADE_NO_NPC,
                unk_e: TRADE_UNK_E,
            })
        };
        // The trade-window inventories come first (capture), for both sides.
        let inventories = |viewer: &Person| {
            TRADE_INVENTORY_IDS.map(|id| {
                SocialOut::AddInventory(AddInventory {
                    unk_10: 1,
                    local_index: viewer.local_index,
                    inventory_id: id,
                    inventory: TRADE_INVENTORY,
                })
            })
        };
        let mut for_a = inventories(&requester).to_vec();
        for_a.push(state(&requester, &other, TRADE_FLAGS_REQUESTER));
        st.send(&a, for_a);
        // The target (verified in game): the requester's entry (the popup looks the name up in the
        // social list), the inventories and the state with the "incoming" flag.
        let mut for_b = vec![SocialOut::Entry(entry(&requester, false))];
        for_b.extend(inventories(&other));
        for_b.push(state(&other, &requester, TRADE_FLAGS_TARGET));
        st.send(&b, for_b);
        Ok(())
    }

    /// `TradeCancel`: either side cancels the trade `trade`. A cancel for a trade that is
    /// already closed is ignored without a chat line: the invitee's client sends its own
    /// `TradeCancel` when its popup is closed by the `TradeAck` of the requester's cancel (seen
    /// in the WP C3 in-game check, 0.25 s later).
    pub fn trade_cancel(&self, from: &str, trade: u32) -> Result<(), String> {
        let mut st = self.state.lock().unwrap();
        let a = from.to_lowercase();
        let Some(pos) = st
            .trades
            .iter()
            .position(|t| t.id == trade && (t.requester == a || t.target == a))
        else {
            return Ok(());
        };
        let t = st.trades.remove(pos);
        let other = if t.requester == a {
            t.target
        } else {
            t.requester
        };
        st.close_trade(trade, &a, true);
        st.close_trade(trade, &other, false);
        Ok(())
    }
}

impl State {
    fn party_of_leader(&self, leader: &str) -> Option<&Party> {
        self.parties.iter().find(|p| p.leader == leader)
    }

    fn send(&self, who: &str, out: Vec<SocialOut>) {
        if let Some(m) = self.people.get(who) {
            for o in out {
                let _ = m.tx.send(o);
            }
        }
    }

    /// `PartyUpdate` as `viewer` sees it: the leader and the invited characters, `viewer`'s own
    /// position in the list.
    fn party_update(
        &self,
        party: u32,
        leader: &Person,
        invited: &[Person],
        viewer: &Person,
        state: u8,
    ) -> PartyUpdate {
        let member = |role: u8, p: &Person| PartyMember {
            role,
            character: p.character.clone(),
            account: p.account.clone(),
            tail: 0xff,
        };
        let mut members = vec![member(ROLE_LEADER, leader)];
        members.extend(invited.iter().map(|p| member(ROLE_INVITED, p)));
        let index = std::iter::once(leader)
            .chain(invited)
            .position(|p| p.character == viewer.character)
            .map_or(0xff, |i| i as u8);
        PartyUpdate {
            local_index: viewer.local_index,
            party,
            unk_a: 0,
            unk_b: 0,
            party2: party,
            unk_0: index,
            unk_1: 1,
            unk_2: 0,
            state,
            members,
        }
    }

    /// The leader `a` takes back (or loses, when the invitee left) the invitation of `b`.
    fn end_invitation(&mut self, a: &str, b: &str) {
        let Some(pos) = self.parties.iter().position(|p| p.leader == a) else {
            return;
        };
        let party = &mut self.parties[pos];
        party.invited.retain(|n| n != b);
        let (id, remaining) = (party.id, party.invited.clone());
        let empty = remaining.is_empty();
        if empty {
            self.parties.remove(pos);
        }
        let person = |n: &str| self.people.get(n).map(|m| m.person.clone());
        // The inviter (capture): the party back to its leader with `state` 1, the invitee's
        // account removed from the list, and when nobody is left the party ended.
        if let (Some(leader), Some(gone)) = (person(a), person(b)) {
            let others: Vec<Person> = remaining.iter().filter_map(|n| person(n)).collect();
            let mut out = vec![
                SocialOut::PartyUpdate(self.party_update(id, &leader, &others, &leader, 1)),
                SocialOut::Removed(SocialRemove {
                    unk_0: 0,
                    account: gone.account.clone(),
                }),
            ];
            if empty {
                out.push(SocialOut::PartyEnded(PartyEnded {
                    local_index: leader.local_index,
                    party: id,
                    reason: ENDED_CANCELLED,
                }));
            }
            self.send(a, out);
            // The invitee (verified in game): the party is over for it.
            self.send(
                b,
                vec![SocialOut::PartyEnded(PartyEnded {
                    local_index: gone.local_index,
                    party: id,
                    reason: ENDED_CANCELLED,
                })],
            );
        }
    }

    /// Ends the trade for `who`: the cancelling player also gets the notice (capture).
    fn close_trade(&self, trade: u32, who: &str, notice: bool) {
        let Some(m) = self.people.get(who) else {
            return;
        };
        let local = m.person.local_index;
        let mut out = Vec::new();
        if notice {
            out.push(SocialOut::Notice(NoticeMessage {
                id: NOTICE_TRADE_CANCELLED,
                unk_a: NOTICE_UNK_A,
                unk_b: NOTICE_UNK_B,
                ..Default::default()
            }));
        }
        out.push(SocialOut::TradeAck(TradeAck {
            local_index: local,
            trade,
            unk: 0,
        }));
        for value in TRADE_EVENT_VALUES {
            out.push(SocialOut::TradeEvent(TradeEvent {
                local_index: local,
                kind: TRADE_EVENT_KIND,
                value,
            }));
        }
        self.send(who, out);
    }

    /// A character left: what it had open ends for the others.
    fn drop_person(&mut self, key: &str) {
        // As the leader: every invitee's invitation ends.
        while let Some(party) = self.parties.iter().find(|p| p.leader == key) {
            let invitee = party.invited[0].clone();
            self.end_invitation(key, &invitee);
        }
        // As an invitee: the leader's invitation to it ends.
        while let Some(leader) = self
            .parties
            .iter()
            .find(|p| p.invited.iter().any(|n| n == key))
            .map(|p| p.leader.clone())
        {
            self.end_invitation(&leader, key);
        }
        // Trades: the other side is told the trade is over.
        while let Some(pos) = self
            .trades
            .iter()
            .position(|t| t.requester == key || t.target == key)
        {
            let t = self.trades.remove(pos);
            let other = if t.requester == key {
                t.target
            } else {
                t.requester
            };
            self.close_trade(t.id, &other, false);
        }
        self.people.remove(key);
    }
}

/// A character's social list entry. `is_self`: yourself, which inside an instance (not a town)
/// has flags 4, no extra words and no second area (capture).
fn entry(p: &Person, is_self: bool) -> SocialEntry {
    let in_instance = is_self && !p.is_town;
    SocialEntry {
        unk_0: 0,
        account: p.account.clone(),
        unk_str: String::new(),
        unk_u64: 0,
        flags: if in_instance {
            ENTRY_SELF_IN_INSTANCE
        } else {
            ENTRY_OTHER
        },
        character: p.character.clone(),
        level: p.level,
        class: p.class,
        area: p.area,
        area2: if in_instance { 0 } else { p.area },
        league: p.league.clone(),
        unk_1fc: ENTRY_UNK_1FC,
        extra_a: (!in_instance).then_some((0, 0)),
        extra_b: None,
        tail: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay_tests::protocol;
    use poe_proto::Direction;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    /// The packet the official server sent: the opcode of `name` and the anonymised body (the
    /// capture's bodies with `Local1`, `Acct#0001`, `Player1`, `remote_acct` for the names, as
    /// in `poe-proto`'s `remote.rs` tests).
    fn official(name: &str, body: &str) -> Vec<u8> {
        let id = protocol().instance.id(Direction::Server, name).unwrap();
        let mut v = id.to_be_bytes().to_vec();
        v.extend(hex(body));
        v
    }

    fn person(
        name: &str,
        account: &str,
        level: u8,
        class: u8,
        area: u16,
        is_town: bool,
        entity: u32,
        session: u64,
    ) -> Person {
        Person {
            character: name.into(),
            account: account.into(),
            level,
            class,
            area,
            is_town,
            league: "Forbidden Rites".into(),
            instance: Some(1),
            entity,
            local_index: 1,
            session,
        }
    }

    /// Alice is the capture's player (`Local1`) in the town, Bob the invitee (`Player1`).
    fn two() -> (
        Social,
        mpsc::UnboundedReceiver<SocialOut>,
        mpsc::UnboundedReceiver<SocialOut>,
    ) {
        // The capture's first party id; its first trade id is 1.
        let social = Social::starting_at(0xcfc43, 1);
        let a = social.register(person("Local1", "Acct#0001", 2, 2, 0x3f70, true, 12013, 1));
        let b = social.register(person(
            "Player1",
            "remote_acct",
            10,
            7,
            0xe911,
            false,
            12002,
            2,
        ));
        (social, a, b)
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<SocialOut>) -> Vec<Vec<u8>> {
        let table = protocol();
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m.encode(&table.instance).unwrap());
        }
        out
    }

    #[test]
    fn an_invitation_sends_the_inviter_the_official_bytes() {
        let (social, mut a, _b) = two();
        social.invite("Local1", "Player1").unwrap();
        let alone =
            "01000cfc4300000000000cfc4300010000010200064c6f63616c310009416363742330303031ff";
        assert_eq!(
            drain(&mut a),
            vec![
                official(
                    "SocialEntry",
                    "000009416363742330303031000000000000000000000500064c6f63616c3102023f703f70000f466f7262696464656e2052697465739b0000000000"
                ),
                official("PartyUpdate", alone),
                official("PartyUpdate", alone),
                official(
                    "SocialEntry",
                    "00000b72656d6f74655f6163637400000000000000000000050007506c61796572310a07e911e911000f466f7262696464656e2052697465739b0000000000"
                ),
                official(
                    "PartyUpdate",
                    "01000cfc4300000000000cfc4300010000020200064c6f63616c310009416363742330303031ff000007506c6179657231000b72656d6f74655f61636374ff"
                ),
            ]
        );
    }

    #[test]
    fn cancelling_an_invitation_sends_the_inviter_the_official_bytes() {
        let (social, mut a, mut b) = two();
        social.invite("Local1", "Player1").unwrap();
        drain(&mut a);
        drain(&mut b);
        social.cancel_invite("Local1", "Player1").unwrap();
        assert_eq!(
            drain(&mut a),
            vec![
                official(
                    "PartyUpdate",
                    "01000cfc4300000000000cfc4300010001010200064c6f63616c310009416363742330303031ff"
                ),
                official("SocialRemove", "00000b72656d6f74655f61636374"),
                official("PartyEnded", "01000cfc4304"),
            ]
        );
        // The invitee's popup is withdrawn too.
        assert_eq!(drain(&mut b), vec![official("PartyEnded", "01000cfc4304")]);
        // A new invitation is a new party (the capture: `0xcfc55`).
        social.invite("Local1", "Player1").unwrap();
        let sent = drain(&mut a);
        assert!(
            sent[1].windows(4).any(|w| w == [0x00, 0x0c, 0xfc, 0x44]),
            "party id 0xcfc44"
        );
    }

    #[test]
    fn the_invitee_gets_the_leader_and_a_party_with_itself_invited() {
        let (social, _a, mut b) = two();
        social.invite("Local1", "Player1").unwrap();
        let sent = drain(&mut b);
        assert_eq!(sent.len(), 2);
        // The leader's entry (flags 5, with the leader's area), then the party with Bob at
        // index 1 as an invited member.
        assert_eq!(
            sent[0],
            official(
                "SocialEntry",
                "000009416363742330303031000000000000000000000500064c6f63616c3102023f703f70000f466f7262696464656e2052697465739b0000000000"
            )
        );
        assert_eq!(
            sent[1],
            official(
                "PartyUpdate",
                "01000cfc4300000000000cfc4301010000020200064c6f63616c310009416363742330303031ff000007506c6179657231000b72656d6f74655f61636374ff"
            )
        );
    }

    #[test]
    fn invitations_that_cannot_work_say_why() {
        let (social, _a, _b) = two();
        assert!(
            social
                .invite("Local1", "Nobody")
                .unwrap_err()
                .contains("not online")
        );
        assert!(
            social
                .invite("Local1", "local1")
                .unwrap_err()
                .contains("yourself")
        );
        social.invite("Local1", "Player1").unwrap();
        assert!(
            social
                .invite("Local1", "Player1")
                .unwrap_err()
                .contains("already")
        );
        assert!(social.cancel_invite("Player1", "Local1").is_err());
        assert!(social.cancel_invite("Local1", "Nobody").is_err());
    }

    #[test]
    fn a_leaving_player_ends_what_it_had_open() {
        let (social, mut a, mut b) = two();
        social.invite("Local1", "Player1").unwrap();
        drain(&mut a);
        drain(&mut b);
        // The invitee leaves: the inviter's invitation is cancelled.
        social.unregister("Player1", 2);
        let sent = drain(&mut a);
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2], official("PartyEnded", "01000cfc4304"));
        // A stale session id does not remove a newer session.
        let _again = social.register(person(
            "Player1",
            "remote_acct",
            10,
            7,
            0xe911,
            false,
            12002,
            3,
        ));
        social.unregister("Player1", 2);
        assert!(social.invite("Local1", "Player1").is_ok());
    }

    #[test]
    fn a_trade_request_reaches_both_and_either_side_cancels() {
        let (social, mut a, mut b) = two();
        // Alice asks the player behind entity 12002 (Bob).
        social.trade_request("Local1", 12002).unwrap();
        // The two trade-window inventories first (the capture's bytes), then the state.
        let inv1 = official("AddInventory", "01010000000149790c054000");
        let inv2 = official("AddInventory", "01010000000249790c054000");
        assert_eq!(
            drain(&mut a),
            vec![
                inv1.clone(),
                inv2.clone(),
                official(
                    "TradeState",
                    "01000000010000000200000001310007506c6179657231ffffffff006a"
                )
            ]
        );
        // Bob: Alice's entry, the inventories (the client throws "Invalid inventory id" while
        // it builds the trade window without them), then the state with the incoming flag
        // (0x32) and Alice's name.
        let sent = drain(&mut b);
        assert_eq!(sent.len(), 4);
        assert_eq!(&sent[1..3], &[inv1, inv2]);
        assert_eq!(
            sent[3],
            official(
                "TradeState",
                "010000000100000002000000013200064c6f63616c31ffffffff006a"
            )
        );
        // Alice cancels (the capture): notice, ack, two events.
        social.trade_cancel("Local1", 1).unwrap();
        assert_eq!(
            drain(&mut a),
            vec![
                official("NoticeMessage", "0000e1ffff0200000000000000"),
                official("TradeAck", "010000000100"),
                official("TradeEvent", "010100000002"),
                official("TradeEvent", "010100000001"),
            ]
        );
        // Bob's window closes the same way, without the notice.
        assert_eq!(
            drain(&mut b),
            vec![
                official("TradeAck", "010000000100"),
                official("TradeEvent", "010100000002"),
                official("TradeEvent", "010100000001"),
            ]
        );
        // A second cancel (the invitee's client answers the closed popup with its own) is
        // ignored: no error, nothing sent.
        assert!(social.trade_cancel("Player1", 1).is_ok());
        assert!(drain(&mut a).is_empty() && drain(&mut b).is_empty());

        // The target can cancel as well.
        social.trade_request("Local1", 12002).unwrap();
        drain(&mut a);
        drain(&mut b);
        social.trade_cancel("Player1", 2).unwrap();
        assert_eq!(drain(&mut b).len(), 4);
        assert_eq!(drain(&mut a).len(), 3);
    }

    #[test]
    fn trades_that_cannot_work_say_why() {
        let (social, mut a, mut b) = two();
        assert!(social.trade_request("Local1", 99999).is_err());
        assert!(
            social
                .trade_request("Local1", 12013)
                .unwrap_err()
                .contains("yourself")
        );
        social.trade_request("Local1", 12002).unwrap();
        assert!(
            social
                .trade_request("Player1", 12013)
                .unwrap_err()
                .contains("already")
        );
        // Leaving cancels the trade for the other side.
        drain(&mut a);
        drain(&mut b);
        social.unregister("Local1", 1);
        assert_eq!(drain(&mut b).len(), 3);
    }
}
