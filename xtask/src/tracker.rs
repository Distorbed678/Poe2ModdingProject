//! `cargo xtask tracker`: renders `docs/status/tracker.toml` into README.md.
//!
//! The README holds a block between [`START`] and [`END`]; everything between the markers is
//! generated. Scores: done = 1, partial = 0.5, missing/planned = 0.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const START: &str = "<!-- tracker:start -->";
pub const END: &str = "<!-- tracker:end -->";
/// Tracker data, relative to the repo root.
pub const TRACKER_PATH: &str = "docs/status/tracker.toml";
/// Where the rendered block goes, relative to the repo root.
pub const README_PATH: &str = "README.md";
const BAR_WIDTH: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Done,
    Partial,
    Missing,
    Planned,
}

impl Status {
    fn icon(self) -> &'static str {
        match self {
            Status::Done => "✅",
            Status::Partial => "🟡",
            Status::Missing => "❌",
            Status::Planned => "⬜",
        }
    }

    fn score(self) -> f64 {
        match self {
            Status::Done => 1.0,
            Status::Partial => 0.5,
            Status::Missing | Status::Planned => 0.0,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub name: String,
    pub status: Status,
    pub link: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subsystem {
    pub name: String,
    pub link: Option<String>,
    #[serde(default, rename = "item")]
    pub items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tracker {
    pub current_task: String,
    pub current_task_link: Option<String>,
    #[serde(default, rename = "subsystem")]
    pub subsystems: Vec<Subsystem>,
}

impl Subsystem {
    /// Fraction of the work done, 0.0 to 1.0.
    pub fn score(&self) -> f64 {
        if self.items.is_empty() {
            return 0.0;
        }
        self.items.iter().map(|i| i.status.score()).sum::<f64>() / self.items.len() as f64
    }
}

/// `█████░░░░░ 50%`
pub fn progress_bar(score: f64) -> String {
    let score = score.clamp(0.0, 1.0);
    let filled = ((score * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    format!(
        "{}{} {}%",
        "█".repeat(filled),
        "░".repeat(BAR_WIDTH - filled),
        (score * 100.0).round() as u32
    )
}

/// GitHub's heading anchor: lowercase, spaces to hyphens, other punctuation dropped.
fn slug(heading: &str) -> String {
    heading
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn linked(text: &str, link: Option<&str>) -> String {
    match link {
        Some(l) => format!("[{}]({l})", cell(text)),
        None => cell(text),
    }
}

/// The generated block, without the markers; ends with a newline.
pub fn render(t: &Tracker) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "> **Current task:** {}\n\n",
        linked(&t.current_task, t.current_task_link.as_deref())
    ));
    out.push_str("| Subsystem | Progress | Done |\n|---|---|---|\n");
    for s in &t.subsystems {
        let done = s.items.iter().filter(|i| i.status == Status::Done).count();
        out.push_str(&format!(
            "| [{}](#{}) | `{}` | {done}/{} |\n",
            cell(&s.name),
            slug(&s.name),
            progress_bar(s.score()),
            s.items.len()
        ));
    }
    out.push_str(
        "\n✅ done · 🟡 partial · ❌ missing · ⬜ planned. \
         Progress counts done as 1 and partial as 0.5.\n",
    );
    for s in &t.subsystems {
        out.push_str(&format!("\n### {}\n\n", s.name));
        if let Some(l) = &s.link {
            out.push_str(&format!("Docs: [{l}]({l})\n\n"));
        }
        out.push_str("| | Item | Note |\n|---|---|---|\n");
        for i in &s.items {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                i.status.icon(),
                linked(&i.name, i.link.as_deref()),
                i.note.as_deref().map(cell).unwrap_or_default()
            ));
        }
    }
    out
}

/// Replaces the text between the markers with `block`.
pub fn splice(readme: &str, block: &str) -> Result<String> {
    let start = readme
        .find(START)
        .with_context(|| format!("README.md has no {START} marker"))?;
    let end = readme
        .find(END)
        .with_context(|| format!("README.md has no {END} marker"))?;
    if end < start {
        bail!("{END} comes before {START}");
    }
    Ok(format!(
        "{}{START}\n{block}{}",
        &readme[..start],
        &readme[end..]
    ))
}

pub fn load(root: &Path) -> Result<Tracker> {
    let path = root.join(TRACKER_PATH);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Renders the README. With `check`, only compares and fails when it is out of date.
pub fn run(root: &Path, check: bool) -> Result<()> {
    let tracker = load(root)?;
    let readme_path = root.join(README_PATH);
    let readme = std::fs::read_to_string(&readme_path)
        .with_context(|| format!("reading {}", readme_path.display()))?;
    let updated = splice(&readme, &render(&tracker))?;
    if updated == readme {
        println!("README.md tracker is up to date");
        return Ok(());
    }
    if check {
        bail!("README.md tracker is out of date; run `cargo xtask tracker`");
    }
    std::fs::write(&readme_path, updated)?;
    println!("README.md tracker updated ({} subsystems)", tracker.subsystems.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
current_task = "Do it"
current_task_link = "docs/plan.md"

[[subsystem]]
name = "Instances/Transfers"
link = "docs/a.md"

[[subsystem.item]]
name = "One"
status = "done"
link = "docs/one.md"

[[subsystem.item]]
name = "Two | pipe"
status = "partial"
note = "half"

[[subsystem.item]]
name = "Three"
status = "missing"

[[subsystem.item]]
name = "Four"
status = "planned"

[[subsystem]]
name = "Empty"
"#;

    fn sample() -> Tracker {
        toml::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn bar_and_percent() {
        assert_eq!(progress_bar(0.0), "░░░░░░░░░░ 0%");
        assert_eq!(progress_bar(0.5), "█████░░░░░ 50%");
        assert_eq!(progress_bar(1.0), "██████████ 100%");
        assert_eq!(progress_bar(0.375), "████░░░░░░ 38%");
        assert_eq!(progress_bar(7.0), "██████████ 100%");
    }

    #[test]
    fn scores() {
        let t = sample();
        assert_eq!(t.subsystems[0].score(), 0.375);
        assert_eq!(t.subsystems[1].score(), 0.0);
    }

    #[test]
    fn renders_banner_summary_and_tables() {
        let md = render(&sample());
        assert!(md.starts_with("> **Current task:** [Do it](docs/plan.md)\n"));
        assert!(md.contains("| [Instances/Transfers](#instancestransfers) | `████░░░░░░ 38%` | 1/4 |"));
        assert!(md.contains("| [Empty](#empty) | `░░░░░░░░░░ 0%` | 0/0 |"));
        assert!(md.contains("### Instances/Transfers\n\nDocs: [docs/a.md](docs/a.md)\n"));
        assert!(md.contains("| ✅ | [One](docs/one.md) |  |"));
        assert!(md.contains("| 🟡 | Two \\| pipe | half |"));
        assert!(md.contains("| ❌ | Three |  |"));
        assert!(md.contains("| ⬜ | Four |  |"));
    }

    #[test]
    fn banner_without_link() {
        let t: Tracker = toml::from_str("current_task = \"X\"").unwrap();
        assert!(render(&t).starts_with("> **Current task:** X\n"));
    }

    #[test]
    fn splice_replaces_only_between_markers_and_is_idempotent() {
        let readme = format!("before\n{START}\nold\n{END}\nafter\n");
        let once = splice(&readme, "new\n").unwrap();
        assert_eq!(once, format!("before\n{START}\nnew\n{END}\nafter\n"));
        assert_eq!(splice(&once, "new\n").unwrap(), once);
    }

    #[test]
    fn splice_needs_markers() {
        assert!(splice("nothing", "x").is_err());
        assert!(splice(&format!("{END}\n{START}"), "x").is_err());
    }

    #[test]
    fn rejects_unknown_status() {
        let bad = "current_task = \"x\"\n[[subsystem]]\nname = \"a\"\n[[subsystem.item]]\nname = \"b\"\nstatus = \"bogus\"\n";
        assert!(toml::from_str::<Tracker>(bad).is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Instances/Transfers"), "instancestransfers");
        assert_eq!(slug("Client Hook"), "client-hook");
        assert_eq!(slug("Monsters/Skills"), "monstersskills");
    }

    #[test]
    fn the_real_tracker_parses_and_renders() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let t = load(root).unwrap();
        assert!(!t.subsystems.is_empty());
        assert!(render(&t).contains("Current task"));
    }
}
