//! The config editor: view, validate and save the file the server was started with.
//!
//! - **Which file.** `Config::source_path`: the file `Config::load_default` read at start
//!   (`POE_CONFIG`, else `config/server.toml`). Nothing else is ever written.
//! - **Validation first.** Text is parsed with `poe-config` (`Config::check_text`: TOML syntax,
//!   the typed sections with ranges, the game version file) before it is saved; an invalid file
//!   is refused with the line and column.
//! - **Backup, then atomic write.** The previous file is copied to `[web] config_backup_dir`
//!   as `<name>.<UTC timestamp>` (the oldest beyond `config_backup_keep` are deleted); the new
//!   text goes to a temporary file next to the original and is renamed over it.
//! - **Conflicts.** The page sends the revision (SHA-256 prefix) of the file it opened; if the
//!   file changed since, the save is refused with `409` so nobody overwrites a hand edit.
//! - **Secrets.** The config holds none. Any string under a token-like key is still hidden in
//!   what the editor shows (`poe_config::edit::redact`) and restored on save.
//! - **Safe defaults.** Saving never loosens anything by itself; dangerous values
//!   (`Config::security_warnings`, e.g. a non-loopback web bind) are returned as warnings. The
//!   committed `config/server.toml` has the safe defaults and a test keeps it so; run a server
//!   that others use from your own copy (`POE_CONFIG=...`), which the editor then edits.
//! - **Audit.** Every save is recorded (`config save`) with the number of changed lines and the
//!   backup name; the contents are never logged.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
    http::StatusCode,
};
use poe_config::{
    Config,
    edit::{self, EditValue},
    schema::{self, FieldKind, FieldSpec},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::{
    ApiError, PendingShutdown, ShutdownKind, WebState, auth::AdminAuth, dto::ShutdownRequest,
    error::ApiJson, lifecycle, public::blocking,
};

/// Largest config text the editor takes.
pub const MAX_CONFIG_BYTES: usize = 1 << 20;
/// Backups listed at most.
const BACKUP_LIST_LIMIT: usize = 100;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `GET /admin/config`: the file, its form schema and its warnings.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConfigFile {
    /// Where the file is.
    pub path: String,
    /// The file's text (token-like values hidden).
    pub content: String,
    /// Identifies this version of the file: send it back as `base_revision` when saving.
    pub revision: String,
    /// `[web] config_editor`: whether saving is allowed.
    pub editable: bool,
    /// Dangerous settings in the file now (`Config::security_warnings`).
    pub warnings: Vec<String>,
    /// Things to know about this file.
    pub notices: Vec<String>,
    /// The values of the form's keys as the file has them (absent: the default applies).
    #[schema(value_type = Object)]
    pub values: serde_json::Value,
    /// The keys the form offers.
    pub fields: Vec<ConfigField>,
    /// Where backups go.
    pub backup_dir: String,
}

/// One key of the form.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConfigField {
    pub section: String,
    pub key: String,
    /// `section.key`.
    pub path: String,
    pub label: String,
    pub help: String,
    /// `bool`, `int`, `text`, `choice` or `list`.
    pub kind: String,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub max_len: Option<usize>,
    pub multiline: bool,
    pub options: Vec<String>,
    /// The value that applies when the key is absent.
    #[schema(value_type = Object)]
    pub default: serde_json::Value,
    /// Whether a restart is needed for a change to apply.
    pub restart: bool,
}

fn to_json(v: &EditValue) -> serde_json::Value {
    match v {
        EditValue::Bool(b) => (*b).into(),
        EditValue::Int(i) => (*i).into(),
        EditValue::Float(f) => (*f).into(),
        EditValue::Text(t) => t.clone().into(),
        EditValue::List(l) => l.clone().into(),
        EditValue::Unset => serde_json::Value::Null,
    }
}

fn from_json(spec: &FieldSpec, v: &serde_json::Value) -> Result<EditValue, String> {
    let wrong = || format!("{}: wrong type of value", spec.label);
    Ok(match (&spec.kind, v) {
        (_, serde_json::Value::Null) => EditValue::Unset,
        (FieldKind::Bool, serde_json::Value::Bool(b)) => EditValue::Bool(*b),
        (FieldKind::Int { .. }, serde_json::Value::Number(n)) => {
            EditValue::Int(n.as_i64().ok_or_else(wrong)?)
        }
        (FieldKind::Text { .. } | FieldKind::Choice(_), serde_json::Value::String(t)) => {
            EditValue::Text(t.clone())
        }
        (FieldKind::List, serde_json::Value::Array(items)) => EditValue::List(
            items
                .iter()
                .map(|i| i.as_str().map(str::to_owned).ok_or_else(wrong))
                .collect::<Result<_, _>>()?,
        ),
        _ => return Err(wrong()),
    })
}

fn field_dto(f: &FieldSpec) -> ConfigField {
    let (kind, min, max, max_len, multiline, options) = match &f.kind {
        FieldKind::Bool => ("bool", None, None, None, false, Vec::new()),
        FieldKind::Int { min, max } => ("int", Some(*min), Some(*max), None, false, Vec::new()),
        FieldKind::Text { multiline, max_len } => {
            ("text", None, None, Some(*max_len), *multiline, Vec::new())
        }
        FieldKind::Choice(o) => (
            "choice",
            None,
            None,
            None,
            false,
            o.iter().map(|s| (*s).to_owned()).collect(),
        ),
        FieldKind::List => ("list", None, None, None, false, Vec::new()),
    };
    ConfigField {
        section: f.section.into(),
        key: f.key.into(),
        path: f.path(),
        label: f.label.into(),
        help: f.help.into(),
        kind: kind.into(),
        min,
        max,
        max_len,
        multiline,
        options,
        default: to_json(&f.default),
        restart: f.restart,
    }
}

/// The revision of a file's text.
pub fn revision(text: &str) -> String {
    hex::encode(&Sha256::digest(text.as_bytes())[..8])
}

fn editor_enabled(s: &WebState) -> Result<(), ApiError> {
    if s.config.web.config_editor {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "the config editor is off ([web] config_editor = false)",
        ))
    }
}

fn source(s: &WebState) -> Result<PathBuf, ApiError> {
    s.config.source_path.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::CONFLICT,
            "this server was not started from a config file",
        )
    })
}

fn read(path: &Path) -> Result<String, ApiError> {
    std::fs::read_to_string(path)
        .map_err(|e| ApiError::internal(format!("reading {}: {e}", path.display())))
}

fn notices(s: &WebState, path: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let committed = Path::new(poe_config::DEFAULT_PATH);
    if path.ends_with(committed)
        && path
            .parent()
            .and_then(Path::parent)
            .is_some_and(|root| root.join(".git").exists())
    {
        out.push(
            "This is the repository's committed config. Saving changes a tracked file, which should keep its safe defaults (loopback only, no registration). Run a server others use from your own copy: POE_CONFIG=runtime/server.toml."
                .into(),
        );
    }
    if !s.config.web.config_editor {
        out.push(
            "The config editor is off ([web] config_editor = false): this page is read-only."
                .into(),
        );
    }
    out.push("Changes to the file apply at the next start, except the message of the day, which applies at once.".into());
    out
}

/// The values the form's keys have in `text` by `section.key`; keys the text leaves out are
/// absent (the default applies). `None` if the text does not parse.
fn form_values(text: &str, fields: &[FieldSpec]) -> Option<serde_json::Value> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let mut values = serde_json::Map::new();
    for f in fields {
        let mut found = table.get(f.section.split('.').next().unwrap_or_default());
        for part in f.section.split('.').skip(1) {
            found = found.and_then(|t| t.get(part));
        }
        if let Some(v) = found.and_then(|t| t.get(f.key)) {
            values.insert(f.path(), serde_json::to_value(v).unwrap_or_default());
        }
    }
    Some(values.into())
}

/// The server's config file, with the form schema.
#[utoipa::path(get, operation_id = "get_config", path = "/api/v1/admin/config", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = ConfigFile), (status = 409, description = "not started from a config file", body = crate::ErrorBody),
        (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn get(State(s): State<WebState>, _: AdminAuth) -> Result<Json<ConfigFile>, ApiError> {
    let path = source(&s)?;
    let state = s.clone();
    let view = blocking(move || -> Result<ConfigFile, ApiError> {
        let text = read(&path)?;
        let shown = edit::redact(&text);
        let fields = schema::common_fields();
        let values = form_values(&shown, &fields)
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
        let warnings = Config::check_text(&text, &state.config_root).warnings;
        Ok(ConfigFile {
            path: path.display().to_string(),
            revision: revision(&text),
            content: shown,
            editable: state.config.web.config_editor,
            warnings,
            notices: notices(&state, &path),
            values,
            fields: fields.iter().map(field_dto).collect(),
            backup_dir: state.config.web.config_backup_dir.display().to_string(),
        })
    })
    .await??;
    Ok(Json(view))
}

/// `POST /admin/config/validate`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ValidateRequest {
    pub content: String,
}

/// A problem in the text.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ConfigProblem {
    pub message: String,
    /// 1-based, when known.
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ValidateResponse {
    pub valid: bool,
    pub error: Option<ConfigProblem>,
    /// Dangerous settings in valid text.
    pub warnings: Vec<String>,
}

fn size_ok(content: &str) -> Result<(), ApiError> {
    if content.len() > MAX_CONFIG_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "the config text is too large",
        ));
    }
    Ok(())
}

fn check(s: &WebState, text: &str) -> ValidateResponse {
    let c = Config::check_text(text, &s.config_root);
    ValidateResponse {
        valid: c.error.is_none(),
        error: c.error.map(|e| ConfigProblem {
            message: e.message,
            line: e.line,
            column: e.column,
        }),
        warnings: c.warnings,
    }
}

/// Parses the text exactly as the server would at start and reports the first problem with its
/// line and column, or the warnings of a valid config. Nothing is saved.
#[utoipa::path(post, operation_id = "validate_config", path = "/api/v1/admin/config/validate", tag = "admin", security(("bearer" = [])),
    request_body = ValidateRequest,
    responses((status = 200, body = ValidateResponse), (status = 413, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn validate(
    State(s): State<WebState>,
    _: AdminAuth,
    ApiJson(req): ApiJson<ValidateRequest>,
) -> Result<Json<ValidateResponse>, ApiError> {
    size_ok(&req.content)?;
    let state = s.clone();
    Ok(Json(blocking(move || check(&state, &req.content)).await?))
}

/// `POST /admin/config/fields`: form values applied to a text.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FieldsRequest {
    /// The text to change (the editor's current content).
    pub content: String,
    /// `section.key` to the new value; `null` removes the key (the default applies again).
    #[schema(value_type = HashMap<String, Object>)]
    pub set: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FieldsResponse {
    /// The text with the values written in; comments and layout are kept.
    pub content: String,
    /// The form values of that text (as `GET /admin/config` reports them), so the form can be
    /// filled from any text; empty when the text does not parse.
    #[schema(value_type = Object)]
    pub values: serde_json::Value,
    /// Why the text does not parse, if it does not.
    pub parse_error: Option<ConfigProblem>,
}

/// Writes form values into config text (comments and layout stay) so the editor can show the
/// result and its diff. Only the keys of the form are accepted; each value is checked against
/// its type and bounds. Nothing is saved. With an empty `set` it only reads the form values out
/// of the text (`parse_error` when it does not parse): the editor uses that to fill the form
/// from text typed in the raw editor.
#[utoipa::path(post, operation_id = "apply_config_fields", path = "/api/v1/admin/config/fields", tag = "admin", security(("bearer" = [])),
    request_body = FieldsRequest,
    responses((status = 200, body = FieldsResponse), (status = 400, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn fields(
    State(_s): State<WebState>,
    _: AdminAuth,
    ApiJson(req): ApiJson<FieldsRequest>,
) -> Result<Json<FieldsResponse>, ApiError> {
    size_ok(&req.content)?;
    let known = schema::common_fields();
    let mut changes = Vec::new();
    let mut keys: Vec<&String> = req.set.keys().collect();
    keys.sort();
    for path in keys {
        let spec = known.iter().find(|f| f.path() == *path).ok_or_else(|| {
            ApiError::bad_request(format!(
                "{path} is not a key of the form: use the raw editor"
            ))
        })?;
        let value = from_json(spec, &req.set[path]).map_err(ApiError::bad_request)?;
        schema::check_value(spec, &value).map_err(ApiError::bad_request)?;
        changes.push((path.clone(), value));
    }
    let fields = schema::common_fields();
    if changes.is_empty() {
        let parse = check_syntax(&req.content);
        return Ok(Json(FieldsResponse {
            values: form_values(&req.content, &fields)
                .unwrap_or_else(|| serde_json::Value::Object(Default::default())),
            parse_error: parse,
            content: req.content,
        }));
    }
    let content = edit::set_values(&req.content, &changes)
        .map_err(|e| ApiError::bad_request(format!("the text does not parse: {e}")))?;
    Ok(Json(FieldsResponse {
        values: form_values(&content, &fields)
            .unwrap_or_else(|| serde_json::Value::Object(Default::default())),
        parse_error: None,
        content,
    }))
}

/// The TOML syntax problem of `text`, if any (without the typed checks).
fn check_syntax(text: &str) -> Option<ConfigProblem> {
    let e = toml::from_str::<toml::Table>(text).err()?;
    let (line, column) = e.span().map_or((None, None), |s| {
        let before = &text[..s.start.min(text.len())];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (Some(line), Some(col))
    });
    Some(ConfigProblem {
        message: e.message().trim().to_owned(),
        line,
        column,
    })
}

/// `POST /admin/config/save`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveRequest {
    /// The whole new text.
    pub content: String,
    /// The `revision` of the file the editor opened.
    pub base_revision: String,
    /// Restart the server after saving (the same options as `POST /admin/server/restart`).
    pub restart: Option<ShutdownRequest>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SaveResponse {
    pub saved: bool,
    /// `false`: the text equals the file, nothing was written.
    pub changed: bool,
    /// File name of the backup of the previous version.
    pub backup: Option<String>,
    /// The new revision.
    pub revision: String,
    /// Lines that differ from the previous version.
    pub lines_changed: usize,
    pub warnings: Vec<String>,
    /// The restart that was requested.
    pub restart: Option<PendingShutdown>,
    /// Why the requested restart did not start (the file is saved anyway).
    pub restart_error: Option<String>,
}

/// How many lines of `new` are not in `old` plus the other way round (a rough size of a change).
pub fn lines_changed(old: &str, new: &str) -> usize {
    let count = |a: &str, b: &str| {
        let mut pool: HashMap<&str, usize> = HashMap::new();
        for l in b.lines() {
            *pool.entry(l).or_default() += 1;
        }
        a.lines()
            .filter(|l| match pool.get_mut(l) {
                Some(n) if *n > 0 => {
                    *n -= 1;
                    false
                }
                _ => true,
            })
            .count()
    };
    count(old, new) + count(new, old)
}

/// What a finished write reports.
pub(crate) struct Written {
    changed: bool,
    backup: Option<String>,
    revision: String,
    lines_changed: usize,
    warnings: Vec<String>,
}

/// Validates `candidate` and replaces the file with it, after a backup. Blocking.
/// `expected`: the revision the caller based the text on (`None`: no check).
fn write_config(
    s: &WebState,
    candidate: &str,
    expected: Option<&str>,
) -> Result<Written, ApiError> {
    editor_enabled(s)?;
    size_ok(candidate)?;
    let path = source(s)?;
    let current = read(&path)?;
    if let Some(expected) = expected
        && expected != revision(&current)
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "the config file changed since you opened it: reload the page and make your change again",
        ));
    }
    let text = edit::restore_hidden(candidate, &current).map_err(ApiError::bad_request)?;
    let verdict = check(s, &text);
    if let Some(e) = verdict.error {
        let at = match (e.line, e.column) {
            (Some(l), Some(c)) => format!("line {l}, column {c}: "),
            (Some(l), None) => format!("line {l}: "),
            _ => String::new(),
        };
        return Err(ApiError::bad_request(format!(
            "the config is not valid, nothing was saved ({at}{})",
            e.message
        )));
    }
    let rev = revision(&current);
    if text == current {
        return Ok(Written {
            changed: false,
            backup: None,
            revision: rev,
            lines_changed: 0,
            warnings: verdict.warnings,
        });
    }
    let backup = backup_file(s, &path, &current)?;
    write_atomic(&path, &text)
        .map_err(|e| ApiError::internal(format!("writing {}: {e}", path.display())))?;
    // The message of the day in the file is the live one.
    if let Ok(new) = Config::parse(&text) {
        s.motd.set(&new.server.motd);
    }
    Ok(Written {
        changed: true,
        backup: Some(backup),
        revision: revision(&text),
        lines_changed: lines_changed(&current, &text),
        warnings: verdict.warnings,
    })
}

fn file_stem(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| "config".into(), |n| n.to_string_lossy().into_owned())
}

/// Copies `previous` into the backup directory and deletes the oldest beyond the limit.
fn backup_file(s: &WebState, path: &Path, previous: &str) -> Result<String, ApiError> {
    use std::io::Write;
    let dir = &s.config.web.config_backup_dir;
    std::fs::create_dir_all(dir)
        .map_err(|e| ApiError::internal(format!("creating {}: {e}", dir.display())))?;
    let stem = file_stem(path);
    let stamp = poe_accounts::format_stamp(now());
    // Several saves in one second get `-001`, `-002`, ...: always after the ones before (a name
    // freed by the trimming below is never reused, which would sort it as the oldest).
    let base = format!("{stem}.{stamp}");
    let mut next = backup_names(dir, &stem)
        .iter()
        .filter_map(|n| n.strip_prefix(&base))
        .filter_map(|rest| {
            if rest.is_empty() {
                Some(0)
            } else {
                rest.strip_prefix('-')?.parse::<u32>().ok()
            }
        })
        .max()
        .map_or(0, |m| m + 1);
    let mut file = loop {
        let name = if next == 0 {
            base.clone()
        } else {
            format!("{base}-{next:03}")
        };
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(dir.join(&name)) {
            Ok(f) => break (f, name),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => next += 1,
            Err(e) => {
                return Err(ApiError::internal(format!(
                    "writing the backup in {}: {e}",
                    dir.display()
                )));
            }
        }
    };
    let name = std::mem::take(&mut file.1);
    let mut file = file.0;
    file.write_all(previous.as_bytes())
        .map_err(|e| ApiError::internal(format!("writing the backup {name}: {e}")))?;
    let keep = s.config.web.config_backup_keep.max(1);
    let mut all = backup_names(dir, &stem);
    all.sort();
    while all.len() > keep {
        let old = all.remove(0);
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(name)
}

fn backup_names(dir: &Path, stem: &str) -> Vec<String> {
    let prefix = format!("{stem}.");
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| {
                    n.starts_with(&prefix)
                        && n[prefix.len()..].starts_with(|c: char| c.is_ascii_digit())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Writes `text` to a temporary file beside `path` and renames it over it; keeps the file's
/// permissions.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".{}.tmp-{}", file_stem(path), std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        if let Ok(meta) = std::fs::metadata(path) {
            f.set_permissions(meta.permissions())?;
        }
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Applies `changes` to the current file and saves it (the message of the day card). Errors
/// carry the reason for the response's `note`.
pub(crate) async fn persist_fields(
    s: &WebState,
    actor: &str,
    changes: Vec<(String, EditValue)>,
) -> Result<(), ApiError> {
    let state = s.clone();
    let actor = actor.to_owned();
    blocking(move || -> Result<(), ApiError> {
        editor_enabled(&state)?;
        let path = source(&state)?;
        let current = read(&path)?;
        let text = edit::set_values(&current, &changes).map_err(ApiError::bad_request)?;
        let done = write_config(&state, &text, Some(&revision(&current)))?;
        if done.changed {
            state.admin.record(
                &actor,
                "config save",
                &format!(
                    "{} lines changed by a form value, backup {}",
                    done.lines_changed,
                    done.backup.as_deref().unwrap_or("-")
                ),
            );
        }
        Ok(())
    })
    .await?
}

/// Saves the text as the server's config file: validates it, keeps a backup of the previous
/// file and replaces it atomically. `400` with the line and column for an invalid config, `403`
/// when `[web] config_editor` is off, `409` when the file changed since `base_revision`. With
/// `restart` the server then restarts like `POST /admin/server/restart`.
#[utoipa::path(post, operation_id = "save_config", path = "/api/v1/admin/config/save", tag = "admin", security(("bearer" = [])),
    request_body = SaveRequest,
    responses((status = 200, body = SaveResponse), (status = 400, body = crate::ErrorBody), (status = 409, body = crate::ErrorBody),
        (status = 413, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn save(
    State(s): State<WebState>,
    AdminAuth(who): AdminAuth,
    ApiJson(req): ApiJson<SaveRequest>,
) -> Result<Json<SaveResponse>, ApiError> {
    // Check the restart options before touching the file.
    let restart = match &req.restart {
        Some(r) => Some(crate::admin::shutdown_options(
            r.delay_secs,
            r.message.as_deref(),
        )?),
        None => None,
    };
    let state = s.clone();
    let candidate = req.content;
    let base = req.base_revision;
    let done = blocking(move || write_config(&state, &candidate, Some(&base))).await??;
    let actor = who.account.username.clone();
    s.admin.record(
        &actor,
        "config save",
        &format!(
            "{} lines changed, backup {}{}",
            done.lines_changed,
            done.backup.as_deref().unwrap_or("-"),
            if restart.is_some() {
                ", restart requested"
            } else {
                ""
            }
        ),
    );
    tracing::info!(account = %actor, changed = done.changed, lines = done.lines_changed, "config file saved through the web api");
    let (pending, restart_error) = match restart {
        None => (None, None),
        Some((delay, message)) => match s.lifecycle.request(
            s.admin.clone(),
            &actor,
            ShutdownKind::Restart,
            delay,
            message.clone(),
        ) {
            Ok(p) => {
                s.admin.record(
                    &actor,
                    "server restart",
                    &format!("in {delay} s after a config save"),
                );
                (Some(p), None)
            }
            Err(lifecycle::Refused::Busy(p)) => (
                None,
                Some(format!(
                    "a {} requested by {} is already pending",
                    p.action.as_str(),
                    p.requested_by
                )),
            ),
            Err(_) => (None, Some("the restart was refused".into())),
        },
    };
    Ok(Json(SaveResponse {
        saved: true,
        changed: done.changed,
        backup: done.backup,
        revision: done.revision,
        lines_changed: done.lines_changed,
        warnings: done.warnings,
        restart: pending,
        restart_error,
    }))
}

/// One backup.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BackupInfo {
    pub name: String,
    pub size: u64,
    /// Unix seconds.
    pub modified: i64,
}

/// The saved previous versions of the config file, newest first.
#[utoipa::path(get, operation_id = "list_config_backups", path = "/api/v1/admin/config/backups", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = Vec<BackupInfo>), (status = 409, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn backups(
    State(s): State<WebState>,
    _: AdminAuth,
) -> Result<Json<Vec<BackupInfo>>, ApiError> {
    let path = source(&s)?;
    let dir = s.config.web.config_backup_dir.clone();
    let list = blocking(move || {
        let stem = file_stem(&path);
        let mut names = backup_names(&dir, &stem);
        names.sort();
        names.reverse();
        names
            .into_iter()
            .take(BACKUP_LIST_LIMIT)
            .filter_map(|name| {
                let meta = std::fs::metadata(dir.join(&name)).ok()?;
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);
                Some(BackupInfo {
                    name,
                    size: meta.len(),
                    modified,
                })
            })
            .collect::<Vec<_>>()
    })
    .await?;
    Ok(Json(list))
}

/// `GET /admin/config/backups/{name}`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BackupContent {
    pub name: String,
    /// The text (token-like values hidden).
    pub content: String,
}

/// The text of one backup, to load into the editor.
#[utoipa::path(get, operation_id = "get_config_backup", path = "/api/v1/admin/config/backups/{name}", tag = "admin", security(("bearer" = [])),
    params(("name" = String, Path, description = "backup file name from the list")),
    responses((status = 200, body = BackupContent), (status = 404, body = crate::ErrorBody), (status = 409, body = crate::ErrorBody), (status = 401, body = crate::ErrorBody), (status = 403, body = crate::ErrorBody)))]
pub async fn backup(
    State(s): State<WebState>,
    _: AdminAuth,
    UrlPath(name): UrlPath<String>,
) -> Result<Json<BackupContent>, ApiError> {
    let path = source(&s)?;
    let dir = s.config.web.config_backup_dir.clone();
    let content = blocking(move || -> Result<BackupContent, ApiError> {
        // Only names the listing would show: no path separators, nothing else in the folder.
        if !backup_names(&dir, &file_stem(&path)).contains(&name) {
            return Err(ApiError::not_found("no such backup"));
        }
        let content = std::fs::read_to_string(dir.join(&name))
            .map_err(|e| ApiError::internal(format!("reading the backup: {e}")))?;
        Ok(BackupContent {
            name,
            content: edit::redact(&content),
        })
    })
    .await??;
    Ok(Json(content))
}
