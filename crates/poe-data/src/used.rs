//! The tables and columns the server and the dev tools read: the list `cargo xtask check-build`
//! verifies against a build's `Bundles2` (the column exists in the schema, the schema row width
//! equals the file's, the values decode) and the anchors of `versions/<version>.toml`.
//!
//! Keep it in step with [`crate::views`], `poe-game` (`Stats`), `poe-world` (`WorldAreas`,
//! `Topologies`), the login server (`BackendErrors`) and `xtask` (the DLL name tables): a new
//! `table("X")` / `.str("Col")` call belongs here. Columns are addressed by name; the few
//! positional reads are in `indexed`.

/// One table and the columns read from it.
#[derive(Debug, Clone, Copy)]
pub struct UsedTable {
    /// Schema name (`data/balance/<lowercase>.datc64`).
    pub name: &'static str,
    /// Columns read by name.
    pub columns: &'static [&'static str],
    /// Unnamed columns read by index (the schema has no name for them).
    pub indexed: &'static [usize],
}

const fn t(name: &'static str, columns: &'static [&'static str]) -> UsedTable {
    UsedTable {
        name,
        columns,
        indexed: &[],
    }
}

/// Every table the server reads.
pub const USED: &[UsedTable] = &[
    t(
        "WorldAreas",
        &[
            "Id",
            "Name",
            "Act",
            "AreaLevel",
            "IsTown",
            "IsHideout",
            "HASH16",
            "Topologies",
        ],
    ),
    t("Topologies", &["DGRFile"]),
    t(
        "Characters",
        &[
            "Id",
            "Name",
            "IntegerId",
            "AOFile",
            "BaseMaxLife",
            "BaseMaxMana",
            "BaseStrength",
            "BaseDexterity",
            "BaseIntelligence",
        ],
    ),
    t(
        "CharacterStartStates",
        &["Id", "Level", "HubArea", "Character"],
    ),
    t("Inventories", &["Id", "InventoryIdKey"]),
    t("LeagueFlag", &["Id"]),
    t("Stats", &["Id"]),
    // Key column by index: the skill id of a basic action is 0x20000000 | key.
    UsedTable {
        name: "ActionTypes",
        columns: &["Id"],
        indexed: &[1],
    },
    t(
        "GrantedEffects",
        &["Id", "StatSet", "AdditionalStatSets", "ActiveSkill"],
    ),
    t("GrantedEffectsPerLevel", &["GrantedEffect", "Level"]),
    t(
        "GrantedEffectStatSets",
        &["ConstantStats", "ConstantStatsValues", "ImplicitStats"],
    ),
    t("GrantedEffectStatSetsPerLevel", &["StatSet", "GemLevel"]),
    t("ActiveSkills", &["ActionType"]),
    t("BackendErrors", &["Id", "Text"]),
    // Rows named by `[movement] sprint_buff_rows`; only the checks read the ids.
    t("BuffDefinitions", &["Id"]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        for (i, a) in USED.iter().enumerate() {
            assert!(USED[i + 1..].iter().all(|b| b.name != a.name), "{}", a.name);
        }
    }
}
