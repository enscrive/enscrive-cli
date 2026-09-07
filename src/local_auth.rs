//! ENS5903: prepared local ESM authority. No key generation or ambient fallback.
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
mod process;

pub(super) const NAMES: [&str; 10] = [
    "OBSERVE_GRPC_CREDENTIALS_VERSION",
    "OBSERVE_GRPC_DEVELOPER_KEY",
    "OBSERVE_GRPC_EMBED_KEY",
    "OBSERVE_GRPC_SENTINEL_KEY",
    "OBSERVE_GRPC_DEVELOPER_PREVIOUS_KEY",
    "OBSERVE_GRPC_EMBED_PREVIOUS_KEY",
    "OBSERVE_GRPC_SENTINEL_PREVIOUS_KEY",
    "OBSERVE_GRPC_DEVELOPER_PREVIOUS_NOT_AFTER",
    "OBSERVE_GRPC_EMBED_PREVIOUS_NOT_AFTER",
    "OBSERVE_GRPC_SENTINEL_PREVIOUS_NOT_AFTER",
];
const SERVICES: [&str; 3] = ["enscrive-developer", "enscrive-observe", "enscrive-embed"];
pub(super) fn recognized(k: &str) -> bool {
    NAMES.contains(&k)
}
fn error(kind: &str) -> String {
    format!("local Observe credentials: {kind}")
}
pub(super) fn path_text(p: &Path) -> Result<&str, String> {
    let s = p.to_str().ok_or_else(|| error("non-Unicode path"))?;
    if s.is_empty() || s.contains(['\n', '\r', '\t', '\0']) || !p.is_absolute() {
        return Err(error("invalid absolute path"));
    }
    Ok(s)
}
pub(super) fn xdg(value: Option<OsString>, fallback: PathBuf) -> Result<PathBuf, String> {
    let p = value.map(PathBuf::from).unwrap_or(fallback);
    path_text(&p)?;
    Ok(p)
}

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    len: u64,
    hash: [u8; 32],
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    mode: u32,
}
fn identity(p: &Path, limit: u64) -> Result<Identity, String> {
    path_text(p)?;
    let m = fs::symlink_metadata(p).map_err(|_| error("input unavailable"))?;
    if !m.is_file() || m.len() > limit {
        return Err(error("invalid input file"));
    }
    let mut f = fs::File::open(p).map_err(|_| error("input unreadable"))?;
    let opened = f
        .metadata()
        .map_err(|_| error("input metadata unavailable"))?;
    #[cfg(unix)]
    if m.dev() != opened.dev() || m.ino() != opened.ino() {
        return Err(error("input changed"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut f)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("input unreadable"))?;
    if bytes.len() as u64 != m.len() || bytes.len() as u64 > limit {
        return Err(error("input changed or oversized"));
    }
    Ok(Identity {
        len: m.len(),
        hash: Sha256::digest(&bytes).into(),
        #[cfg(unix)]
        dev: m.dev(),
        #[cfg(unix)]
        ino: m.ino(),
        #[cfg(unix)]
        mode: m.mode(),
    })
}
pub(super) struct Executable {
    path: PathBuf,
    identity: Identity,
}
impl Executable {
    pub(super) fn resolve(explicit: Option<&str>, path: Option<OsString>) -> Result<Self, String> {
        if let Some(s) = explicit {
            return Self::at(Path::new(s));
        }
        let paths = path.ok_or_else(|| {
            error("ESM missing: install esm or supply --esm-bin with an absolute executable path")
        })?;
        for dir in std::env::split_paths(&paths) {
            path_text(&dir)?;
            let p = dir.join("esm");
            match fs::symlink_metadata(&p) {
                Ok(_) => return Self::at(&p),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(error("ESM discovery unavailable")),
            }
        }
        Err(error(
            "ESM missing: install esm or supply --esm-bin with an absolute executable path",
        ))
    }
    pub(super) fn at(p: &Path) -> Result<Self, String> {
        path_text(p)?;
        // Installed symlinks are resolved once, then the target path and bytes are frozen.
        let p = fs::canonicalize(p).map_err(|_| error("ESM executable unavailable"))?;
        path_text(&p)?;
        let id = identity(&p, 512 * 1024 * 1024)?;
        #[cfg(unix)]
        if id.mode & 0o111 == 0 || !process::executable(&p) {
            return Err(error("ESM path is not executable"));
        }
        Ok(Self {
            path: p,
            identity: id,
        })
    }
    pub(super) fn text(&self) -> &str {
        self.path.to_str().expect("validated path")
    }
    pub(super) fn verify(&self) -> Result<(), String> {
        if identity(&self.path, 512 * 1024 * 1024)? != self.identity {
            Err(error("ESM executable changed"))
        } else {
            Ok(())
        }
    }
}

pub(super) struct Recipient {
    service: &'static str,
    values: BTreeMap<String, String>,
}
impl std::fmt::Debug for Recipient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Recipient([REDACTED])")
    }
}
impl Recipient {
    pub(super) fn require_service(&self, service: &str) -> Result<(), String> {
        if self.service == service {
            Ok(())
        } else {
            Err(error("recipient service mismatch"))
        }
    }
    pub(super) fn apply(&self, cmd: &mut Command) {
        for k in NAMES {
            cmd.env_remove(k);
        }
        for (k, v) in &self.values {
            cmd.env(k, v);
        }
    }
    pub(super) fn refresh(&self, doc: &mut EnvDocument) {
        doc.lines.retain(|l| match l.split_once('=') {
            Some((k, _)) => !recognized(k.trim()),
            None => true,
        });
        for (k, v) in &self.values {
            doc.lines.push(format!("{k}={v}"));
        }
    }
}
pub(super) struct Prepared {
    executable: Executable,
    developer: Recipient,
    observe: Recipient,
    embed: Recipient,
}
impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Prepared([REDACTED])")
    }
}
impl Prepared {
    pub(super) fn executable(&self) -> &Executable {
        &self.executable
    }
    pub(super) fn developer(&self) -> &Recipient {
        &self.developer
    }
    pub(super) fn observe(&self) -> &Recipient {
        &self.observe
    }
    pub(super) fn embed(&self) -> &Recipient {
        &self.embed
    }
    pub(super) fn load(
        executable: Executable,
        runtime: &Path,
        now: DateTime<Utc>,
    ) -> Result<Self, String> {
        let deadline = Instant::now() + Duration::from_secs(180);
        path_text(runtime)?;
        let master = runtime.join(".esm-master-key");
        let vaults = SERVICES.map(|s| runtime.join("secrets").join(s).join(".esm/secrets.esm"));
        for p in std::iter::once(&master).chain(vaults.iter()) {
            path_text(p)?;
            match fs::symlink_metadata(p) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(format!(
                        "local Observe credentials: prepare existing file {} before retry; each vault uses its service directory as cwd",
                        path_text(p)?
                    ));
                }
                Err(_) => return Err(error("prepared input unavailable")),
                Ok(_) => {}
            }
        }
        let master_id = identity(&master, 8192)?;
        let key = fs::read_to_string(&master).map_err(|_| error("master file unavailable"))?;
        if key.trim().is_empty() {
            return Err(error("empty prepared master file"));
        }
        let before = vaults
            .iter()
            .map(|p| identity(p, 64 * 1024 * 1024))
            .collect::<Result<Vec<_>, _>>()?;
        let mut maps = Vec::new();
        for (i, vault) in vaults.iter().enumerate() {
            let cwd = runtime.join("secrets").join(SERVICES[i]);
            path_text(&cwd)?;
            executable.verify()?;
            if Instant::now() >= deadline {
                return Err(error("inventory deadline exceeded"));
            }
            let output = process::capture(
                &executable.path,
                &cwd,
                &[
                    "vault",
                    "check-passphrase",
                    "--key-file",
                    path_text(&master)?,
                    path_text(vault)?,
                ],
                8192,
                deadline,
            )?;
            if output != format!("open\t{}\n", path_text(vault)?).as_bytes() {
                return Err(error("vault authentication unavailable"));
            }
            let output = process::capture(
                &executable.path,
                &cwd,
                &[
                    "--vault-path",
                    path_text(vault)?,
                    "--no-keyring",
                    "list",
                    "--json",
                    "--show-derived",
                ],
                1024 * 1024,
                deadline,
            )?;
            let names = parse_list(&output)?;
            let mut map = BTreeMap::new();
            for name in names {
                let output = process::capture(
                    &executable.path,
                    &cwd,
                    &[
                        "--vault-path",
                        path_text(vault)?,
                        "--no-keyring",
                        "--key-file",
                        path_text(&master)?,
                        "get",
                        name,
                        "--raw",
                    ],
                    1024,
                    deadline,
                )?;
                let value =
                    String::from_utf8(output).map_err(|_| error("invalid raw get output"))?;
                map.insert(name.to_string(), value);
            }
            maps.push(map);
        }
        executable.verify()?;
        if identity(&master, 8192)? != master_id {
            return Err(error("master input changed"));
        }
        for (v, id) in vaults.iter().zip(before) {
            if Instant::now() >= deadline {
                return Err(error("inventory deadline exceeded"));
            }
            if identity(v, 64 * 1024 * 1024)? != id {
                return Err(error("vault input changed"));
            }
        }
        let [developer, observe, embed]: [BTreeMap<String, String>; 3] =
            maps.try_into().map_err(|_| error("inventory incomplete"))?;
        let prepared = Self::validate(executable, developer, observe, embed, now)?;
        if Instant::now() >= deadline {
            return Err(error("inventory deadline exceeded"));
        }
        Ok(prepared)
    }
    fn validate(
        executable: Executable,
        developer: BTreeMap<String, String>,
        observe: BTreeMap<String, String>,
        embed: BTreeMap<String, String>,
        now: DateTime<Utc>,
    ) -> Result<Self, String> {
        let required = |map: &BTreeMap<String, String>, k: &str| {
            map.get(k)
                .cloned()
                .ok_or_else(|| error(&format!("required prepared field missing: {k}")))
        };
        // The marker is rendered, not a required durable secret. If present it must agree.
        for map in [&developer, &observe, &embed] {
            if map.get(NAMES[0]).is_some_and(|v| v != "1") {
                return Err(error("invalid prepared marker"));
            }
        }
        for (map, own) in [(&developer, NAMES[1]), (&embed, NAMES[2])] {
            for k in map.keys() {
                if k != own && k != NAMES[0] {
                    return Err(error("credential in wrong durable vault"));
                }
            }
        }
        let mut seen = BTreeSet::new();
        for name in &NAMES[1..4] {
            let v = required(&observe, name)?;
            validate_key(&v)?;
            if !seen.insert(v) {
                return Err(error("duplicate role credential"));
            }
        }
        for i in 0..3 {
            match (observe.get(NAMES[i + 4]), observe.get(NAMES[i + 7])) {
                (None, None) => {}
                (Some(k), Some(t)) => {
                    validate_key(k)?;
                    if !seen.insert(k.clone()) {
                        return Err(error("duplicate rotation credential"));
                    }
                    let expiry = DateTime::parse_from_rfc3339(t)
                        .map_err(|_| error("invalid previous expiry"))?;
                    if expiry.offset().local_minus_utc() != 0
                        || expiry.with_timezone(&Utc) > now + ChronoDuration::hours(24)
                    {
                        return Err(error("invalid previous expiry"));
                    }
                }
                _ => return Err(error("partial previous credential")),
            }
        }
        if required(&developer, NAMES[1])? != required(&observe, NAMES[1])?
            || required(&embed, NAMES[2])? != required(&observe, NAMES[2])?
        {
            return Err(error("prepared credential pair conflict"));
        }
        let recipient = |service, mut values: BTreeMap<String, String>| {
            values.insert(NAMES[0].into(), "1".into());
            Recipient { service, values }
        };
        Ok(Self {
            executable,
            developer: recipient(SERVICES[0], developer),
            observe: recipient(SERVICES[1], observe),
            embed: recipient(SERVICES[2], embed),
        })
    }
}
fn validate_key(s: &str) -> Result<(), String> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Err(error("invalid prepared key"))
    } else {
        Ok(())
    }
}
fn parse_list(bytes: &[u8]) -> Result<Vec<&'static str>, String> {
    let v: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| error("invalid list output"))?;
    let rows = v.as_array().ok_or_else(|| error("invalid list output"))?;
    let mut found = BTreeSet::new();
    for row in rows {
        let name = row
            .get("name")
            .and_then(|x| x.as_str())
            .ok_or_else(|| error("invalid list record"))?;
        if let Some(&name) = NAMES.iter().find(|&&k| k == name)
            && (row.get("derived").and_then(|x| x.as_bool()) != Some(false) || !found.insert(name))
        {
            return Err(error("derived or duplicate credential record"));
        }
    }
    Ok(found.into_iter().collect())
}

/// Preserves original comments and unrelated assignments; errors never quote lines.
pub(super) struct EnvDocument {
    lines: Vec<String>,
}
impl EnvDocument {
    pub(super) fn parse(text: &str) -> Result<Self, String> {
        let mut seen = BTreeSet::new();
        for line in text.lines() {
            let s = line.trim();
            if s.is_empty() || s.starts_with('#') {
                continue;
            }
            let (k, _) = s
                .split_once('=')
                .ok_or_else(|| error("malformed runtime environment"))?;
            if k.is_empty() || !k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                return Err(error("malformed runtime environment"));
            }
            if recognized(k) && !seen.insert(k) {
                return Err(error("duplicate runtime credential"));
            }
        }
        Ok(Self {
            lines: text.lines().map(str::to_owned).collect(),
        })
    }
    pub(super) fn read(p: &Path) -> Result<Self, String> {
        check_file_if_present(p)?;
        let s = fs::read_to_string(p).map_err(|_| error("runtime environment unavailable"))?;
        Self::parse(&s)
    }
    pub(super) fn optional(p: &Path) -> Result<Self, String> {
        match fs::symlink_metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::parse(""),
            Err(_) => Err(error("runtime environment unavailable")),
            Ok(_) => Self::read(p),
        }
    }
    pub(super) fn entries(&self) -> Vec<(String, String)> {
        self.lines
            .iter()
            .filter_map(|l| {
                let s = l.trim();
                if s.starts_with('#') {
                    None
                } else {
                    s.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned()))
                }
            })
            .collect()
    }
    pub(super) fn replace(&mut self, k: &str, v: &str) {
        let mut found = false;
        for line in &mut self.lines {
            if line
                .trim()
                .split_once('=')
                .is_some_and(|(name, _)| name == k)
            {
                *line = format!("{k}={v}");
                found = true;
            }
        }
        if !found {
            self.lines.push(format!("{k}={v}"));
        }
    }
    pub(super) fn write(&self, p: &Path) -> Result<(), String> {
        protected_write(p, &format!("{}\n", self.lines.join("\n")))
    }
    pub(super) fn merge_rendered(&mut self, text: &str) -> Result<(), String> {
        for (k, v) in Self::parse(text)?.entries() {
            if !recognized(&k) {
                self.replace(&k, &v);
            }
        }
        Ok(())
    }
}
#[cfg(unix)]
fn owned(m: &fs::Metadata) -> bool {
    m.uid() == process::uid()
}
#[cfg(not(unix))]
fn owned(_: &fs::Metadata) -> bool {
    false
}
fn check_file_if_present(p: &Path) -> Result<(), String> {
    path_text(p)?;
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_file() && owned(&m) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(error("unsafe runtime file")),
    }
}
pub(super) fn protected_dir(p: &Path) -> Result<(), String> {
    path_text(p)?;
    // Inspect every existing ancestor for symlinks, but chmod only the owned target.
    for a in p.ancestors() {
        if let Ok(m) = fs::symlink_metadata(a)
            && !m.is_dir()
        {
            return Err(error("unsafe directory ancestor"));
        }
    }
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() && owned(&m) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = p.parent().ok_or_else(|| error("invalid directory"))?;
            if !parent.is_dir() {
                protected_dir(parent)?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(p)
                    .map_err(|_| error("create protected directory failed"))?;
            }
            #[cfg(not(unix))]
            return Err(error("unsupported platform"));
        }
        _ => return Err(error("unsafe runtime directory")),
    }
    #[cfg(unix)]
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))
        .map_err(|_| error("protect directory failed"))?;
    Ok(())
}
pub(super) fn protected_write(p: &Path, s: &str) -> Result<(), String> {
    path_text(p)?;
    check_file_if_present(p)?;
    let parent = p.parent().ok_or_else(|| error("missing parent"))?;
    protected_dir(parent)?;
    let temp = parent.join(format!(".ens5903-{:016x}.tmp", rand::random::<u64>()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut f = options
            .open(&temp)
            .map_err(|_| error("protected write failed"))?;
        f.write_all(s.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|_| error("protected write failed"))?;
        check_file_if_present(p)?;
        fs::rename(&temp, p).map_err(|_| error("protected replacement failed"))?;
        fs::File::open(parent)
            .and_then(|d| d.sync_all())
            .map_err(|_| error("directory sync failed"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub(super) fn stopped(log: &Path) -> Result<(), String> {
    for s in SERVICES {
        process::stopped(&log.join(format!("{s}.pid")))?;
    }
    Ok(())
}

pub(super) fn stopped_service(log: &Path, service: &str) -> Result<(), String> {
    process::stopped(&log.join(format!("{service}.pid")))
}

#[cfg(test)]
pub(super) mod fixtures {
    use super::*;
    pub(crate) fn actual_command(
        binary: &Path,
        cwd: &Path,
        args: &[&str],
    ) -> Result<Vec<u8>, String> {
        process::capture(
            binary,
            cwd,
            args,
            1024 * 1024,
            Instant::now() + Duration::from_secs(20),
        )
    }
    // This is an explicit protocol fake, not an ESM cryptography proof. Every
    // legacy init fixture still traverses Prepared::load and the real gate.
    pub(crate) fn prepare(runtime: &Path) -> PathBuf {
        fs::create_dir_all(runtime).unwrap();
        fs::write(runtime.join(".esm-master-key"), "  synthetic-master\n").unwrap();
        for service in SERVICES {
            let dir = runtime.join("secrets").join(service).join(".esm");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("secrets.esm"),
                b"synthetic unchanged encrypted-file witness",
            )
            .unwrap();
        }
        let binary = runtime.join("fixture-esm");
        fs::write(&binary,r#"#!/usr/bin/python3
import os,sys,json,pathlib
a=sys.argv[1:];service=pathlib.Path.cwd().name
keys={'enscrive-developer':{'OBSERVE_GRPC_DEVELOPER_KEY':'01'*32},'enscrive-embed':{'OBSERVE_GRPC_EMBED_KEY':'02'*32},'enscrive-observe':{'OBSERVE_GRPC_DEVELOPER_KEY':'01'*32,'OBSERVE_GRPC_EMBED_KEY':'02'*32,'OBSERVE_GRPC_SENTINEL_KEY':'03'*32}}
if service not in keys:raise SystemExit(2)
values=keys[service]
if 'check-passphrase' in a:
 assert a[:3]==['vault','check-passphrase','--key-file'] and len(a)==5
 assert pathlib.Path(a[3]).read_text().strip()=='synthetic-master'
 assert pathlib.Path(a[4])==pathlib.Path.cwd()/'.esm/secrets.esm'
 print('open\t'+a[4])
elif 'list' in a:
 assert a==['--vault-path',str(pathlib.Path.cwd()/'.esm/secrets.esm'),'--no-keyring','list','--json','--show-derived']
 print(json.dumps([{'name':k,'derived':False} for k in values]))
elif 'get' in a:
 assert a[:3]==['--vault-path',str(pathlib.Path.cwd()/'.esm/secrets.esm'),'--no-keyring']
 assert a[3]=='--key-file' and a[5]=='get' and a[7]=='--raw' and len(a)==8
 assert pathlib.Path(a[4]).read_text().strip()=='synthetic-master'
 sys.stdout.write(values[a[6]])
elif a and a[0]=='set':
 assert a[3] not in values and a[3] not in {'OBSERVE_GRPC_CREDENTIALS_VERSION', 'OBSERVE_GRPC_DEVELOPER_KEY', 'OBSERVE_GRPC_EMBED_KEY', 'OBSERVE_GRPC_SENTINEL_KEY', 'OBSERVE_GRPC_DEVELOPER_PREVIOUS_KEY', 'OBSERVE_GRPC_EMBED_PREVIOUS_KEY', 'OBSERVE_GRPC_SENTINEL_PREVIOUS_KEY', 'OBSERVE_GRPC_DEVELOPER_PREVIOUS_NOT_AFTER', 'OBSERVE_GRPC_EMBED_PREVIOUS_NOT_AFTER', 'OBSERVE_GRPC_SENTINEL_PREVIOUS_NOT_AFTER'}
 with (pathlib.Path.cwd()/'unrelated-set-keys').open('a') as f:f.write(a[3]+'\n')
else:raise SystemExit(2)
"#).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        binary
    }
}
#[cfg(test)]
mod tests;
