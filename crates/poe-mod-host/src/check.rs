//! The login check of the client's mods: the client hook reports the mods it has loaded
//! (`poe_hook_shared::modreport`, side channel `Kind::ModReport`), the server compares that
//! with the client mods it requires. A consistency check, not anti-cheat: a modified client can
//! report anything.

use poe_hook_shared::modreport::{ModReport, hex};

/// A client mod the server requires: what the installed mod must look like on the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredMod {
    pub id: String,
    pub name: String,
    pub version: String,
    /// SHA-256 of the mod's `mod.toml` (lower-case hex).
    pub manifest_sha256: String,
    /// SHA-256 of the client entry DLL, if the mod has one.
    pub dll_sha256: Option<String>,
}

/// What the check decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    /// The login is refused; one line per problem, ready to show in chat.
    Refused(Vec<String>),
}

/// Compares the client's report with the required set.
///
/// - No report (an old hook, or none): refused when mods are required, allowed otherwise.
/// - A required mod that is missing, has another version, or the same version with another
///   `mod.toml` or DLL hash: refused.
/// - A reported mod that is not required: refused unless `allow_extra`.
pub fn check_client(
    required: &[RequiredMod],
    allow_extra: bool,
    report: Option<&ModReport>,
) -> Verdict {
    let Some(report) = report else {
        if required.is_empty() {
            return Verdict::Allowed;
        }
        let mut lines = vec![
            "This server requires client mods, but your game did not report its mods (update poe-hook / the launcher).".to_owned(),
        ];
        lines.extend(
            required
                .iter()
                .map(|m| format!("needed: {} {}", m.id, m.version)),
        );
        return Verdict::Refused(lines);
    };
    let mut problems = Vec::new();
    for need in required {
        match report.mods.iter().find(|m| m.id == need.id) {
            None => problems.push(format!("missing mod: {} {}", need.id, need.version)),
            Some(have) if have.version != need.version => problems.push(format!(
                "wrong version of {}: you have {}, the server needs {}",
                need.id, have.version, need.version
            )),
            Some(have) if hex(&have.manifest_sha256) != need.manifest_sha256 => {
                problems.push(format!(
                    "{} {} differs from the server's copy (mod.toml changed); reinstall it",
                    need.id, need.version
                ))
            }
            Some(have) => {
                let have_dll = have.dll_sha256.as_ref().map(hex);
                if have_dll != need.dll_sha256 {
                    problems.push(format!(
                        "{} {} differs from the server's copy (its DLL changed); reinstall it",
                        need.id, need.version
                    ));
                }
            }
        }
    }
    if !allow_extra {
        for have in &report.mods {
            if !required.iter().any(|m| m.id == have.id) {
                problems.push(format!(
                    "this server does not allow the client mod {} {}",
                    have.id, have.version
                ));
            }
        }
    }
    if problems.is_empty() {
        Verdict::Allowed
    } else {
        Verdict::Refused(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poe_hook_shared::modreport::{ReportedMod, sha256};

    fn need(id: &str, version: &str, toml: &[u8], dll: Option<&[u8]>) -> RequiredMod {
        RequiredMod {
            id: id.into(),
            name: id.into(),
            version: version.into(),
            manifest_sha256: hex(&sha256(toml)),
            dll_sha256: dll.map(|d| hex(&sha256(d))),
        }
    }

    fn have(id: &str, version: &str, toml: &[u8], dll: Option<&[u8]>) -> ReportedMod {
        ReportedMod {
            id: id.into(),
            version: version.into(),
            manifest_sha256: sha256(toml),
            dll_sha256: dll.map(sha256),
        }
    }

    fn report(mods: Vec<ReportedMod>) -> ModReport {
        ModReport { mods }
    }

    #[test]
    fn nothing_required_and_nothing_reported_is_fine() {
        assert_eq!(
            check_client(&[], false, Some(&report(vec![]))),
            Verdict::Allowed
        );
        // An old hook sends no report: allowed when nothing is required.
        assert_eq!(check_client(&[], false, None), Verdict::Allowed);
    }

    #[test]
    fn an_old_hook_is_refused_when_mods_are_required() {
        let required = [need("a-mod", "1.0.0", b"t", Some(b"d"))];
        let Verdict::Refused(lines) = check_client(&required, true, None) else {
            panic!("should be refused");
        };
        assert!(lines[0].contains("did not report"));
        assert!(lines.iter().any(|l| l.contains("a-mod 1.0.0")));
    }

    #[test]
    fn an_exact_match_passes() {
        let required = [
            need("a-mod", "1.0.0", b"t", Some(b"d")),
            need("data-mod", "2.0.0", b"u", None),
        ];
        let r = report(vec![
            have("data-mod", "2.0.0", b"u", None),
            have("a-mod", "1.0.0", b"t", Some(b"d")),
        ]);
        assert_eq!(check_client(&required, false, Some(&r)), Verdict::Allowed);
    }

    #[test]
    fn missing_wrong_version_and_modified_mods_are_listed() {
        let required = [
            need("a-mod", "1.0.0", b"t", Some(b"d")),
            need("b-mod", "1.0.0", b"t", Some(b"d")),
            need("c-mod", "1.0.0", b"t", Some(b"d")),
            need("d-mod", "1.0.0", b"t", Some(b"d")),
        ];
        let r = report(vec![
            have("b-mod", "0.9.0", b"t", Some(b"d")),
            have("c-mod", "1.0.0", b"CHANGED", Some(b"d")),
            have("d-mod", "1.0.0", b"t", Some(b"patched")),
        ]);
        let Verdict::Refused(lines) = check_client(&required, true, Some(&r)) else {
            panic!("should be refused");
        };
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[0].starts_with("missing mod: a-mod"));
        assert!(lines[1].contains("you have 0.9.0, the server needs 1.0.0"));
        assert!(lines[2].contains("mod.toml changed"));
        assert!(lines[3].contains("DLL changed"));
    }

    #[test]
    fn a_dll_where_none_is_expected_or_the_reverse_differs() {
        let required = [need("a-mod", "1.0.0", b"t", None)];
        let r = report(vec![have("a-mod", "1.0.0", b"t", Some(b"d"))]);
        assert!(matches!(
            check_client(&required, true, Some(&r)),
            Verdict::Refused(_)
        ));
        let required = [need("a-mod", "1.0.0", b"t", Some(b"d"))];
        let r = report(vec![have("a-mod", "1.0.0", b"t", None)]);
        assert!(matches!(
            check_client(&required, true, Some(&r)),
            Verdict::Refused(_)
        ));
    }

    #[test]
    fn extra_client_mods_follow_the_option() {
        let required = [need("a-mod", "1.0.0", b"t", None)];
        let r = report(vec![
            have("a-mod", "1.0.0", b"t", None),
            have("mine", "0.1.0", b"x", Some(b"y")),
        ]);
        assert_eq!(check_client(&required, true, Some(&r)), Verdict::Allowed);
        let Verdict::Refused(lines) = check_client(&required, false, Some(&r)) else {
            panic!("extra mod should be refused");
        };
        assert_eq!(
            lines,
            ["this server does not allow the client mod mine 0.1.0"]
        );
        // With nothing required, extras alone are what is refused.
        assert!(matches!(
            check_client(&[], false, Some(&r)),
            Verdict::Refused(_)
        ));
        assert_eq!(check_client(&[], true, Some(&r)), Verdict::Allowed);
    }
}
