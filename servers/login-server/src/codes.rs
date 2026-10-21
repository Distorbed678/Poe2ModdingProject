//! The client's result code table (`Data/Balance/BackendErrors.dat`) next to the codes the
//! server sends (`poe_proto::backend_error`, code = row + 1).

use poe_data::GameData;
use poe_proto::backend_error::{self, BackendError};

/// Table column `Id` of the row for `code`, or `None` when the table has no such row.
pub fn row_id(data: &GameData, code: u16) -> Option<String> {
    let table = data.table("BackendErrors").ok()?;
    table.row(usize::from(code).checked_sub(1)?)?.str("Id").ok()
}

/// The English text the client prints for `code` (for logs and tools; the wire carries none).
pub fn text(data: &GameData, code: u16) -> Option<String> {
    let table = data.table("BackendErrors").ok()?;
    table
        .row(usize::from(code).checked_sub(1)?)?
        .str("Text")
        .ok()
}

/// Compares every named code with the table. Returns the mismatches (empty = all good); a
/// client update that moves rows shows up here instead of as the wrong text in game.
pub fn verify(data: &GameData) -> Vec<String> {
    mismatches(backend_error::ALL, |code| row_id(data, code))
}

fn mismatches(codes: &[BackendError], id_of: impl Fn(u16) -> Option<String>) -> Vec<String> {
    codes
        .iter()
        .filter_map(|c| match id_of(c.code) {
            Some(id) if id == c.id => None,
            Some(id) => Some(format!(
                "code {} should be {} but the table row is {id}",
                c.code, c.id
            )),
            None => Some(format!(
                "code {} ({}) has no row in BackendErrors",
                c.code, c.id
            )),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatches_are_reported() {
        let codes = [
            backend_error::INVALID_PASSWORD,
            backend_error::LOGIN_RATE_LIMITED,
        ];
        assert!(
            mismatches(&codes, |c| Some(
                codes.iter().find(|b| b.code == c).unwrap().id.to_owned()
            ))
            .is_empty()
        );
        let bad = mismatches(&codes, |c| (c == 6).then(|| "Other".to_owned()));
        assert_eq!(bad.len(), 2);
        assert!(bad[0].contains("InvalidPassword") && bad[1].contains("no row"));
    }

    /// Against the real table (skipped without `GameFiles/`).
    #[test]
    fn named_codes_match_backend_errors_dat() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (game, schema) = (
            root.join("GameFiles"),
            root.join("third_party/dat-schema/schema-poe2.min.json"),
        );
        if !game.join("Bundles2/_.index.bin").exists() || !schema.exists() {
            eprintln!("skipping: GameFiles or schema not present");
            return;
        }
        let data = GameData::open(&game, &schema).unwrap();
        assert_eq!(verify(&data), Vec::<String>::new());
        assert!(
            text(&data, backend_error::LOGIN_RATE_LIMITED.code)
                .unwrap()
                .contains("too often")
        );
    }
}
