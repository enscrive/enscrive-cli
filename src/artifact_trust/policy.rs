use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Provenance {
    pub(crate) authority: String,
    pub(crate) aggregate_sha256: Option<String>,
    pub(crate) component_sha256: Option<String>,
    pub(crate) generation: Option<String>,
    pub(crate) archive: bool,
    pub(crate) pinned: bool,
}
impl Provenance {
    pub(crate) fn local() -> Self {
        Self {
            authority: "operator-trusted".into(),
            aggregate_sha256: None,
            component_sha256: None,
            generation: None,
            archive: false,
            pinned: false,
        }
    }
}
pub(crate) struct Policy {
    pub(crate) manifest: Option<String>,
    pub(crate) expected: Option<String>,
    pub(crate) origin: Option<String>,
}
impl Policy {
    pub(crate) fn new(
        manifest: Option<String>,
        expected: Option<String>,
        origin: Option<String>,
    ) -> Result<Self> {
        let manifest = read_input(manifest, "ENSCRIVE_MANIFEST_URL")?;
        let expected = read_input(expected, "ENSCRIVE_EXPECTED_MANIFEST_SHA256")?;
        let origin = read_input(origin, "ENSCRIVE_PINSET_ORIGIN")?;
        if expected.as_deref().is_some_and(|s| !valid_digest(s)) {
            return Err(fail(
                "expected manifest SHA must be 64 lowercase hex characters",
            ));
        }
        if let Some(v) = &manifest {
            Location::parse(v)?;
        }
        if let Some(v) = &origin {
            Location::parse(v)?;
        }
        Ok(Self {
            manifest,
            expected,
            origin,
        })
    }
    pub(crate) fn explicit(&self) -> bool {
        self.manifest.is_some() || self.expected.is_some() || self.origin.is_some()
    }
}
fn read_input(value: Option<String>, name: &str) -> Result<Option<String>> {
    let v = match value {
        Some(s) => Some(s),
        None => std::env::var_os(name)
            .map(|v| {
                v.into_string()
                    .map_err(|_| fail("release policy input is non-Unicode"))
            })
            .transpose()?,
    };
    if v.as_deref().is_some_and(|s| s.is_empty()) {
        return Err(fail("release policy input is empty"));
    }
    Ok(v)
}
#[derive(Clone)]
pub(super) enum Location {
    Https(String),
    File(PathBuf),
}
impl Location {
    pub(super) fn parse(raw: &str) -> Result<Self> {
        if raw.trim() != raw || raw.bytes().any(|b| b.is_ascii_control()) {
            return Err(fail("release URL lexical whitespace/control refused"));
        }
        if let Some(s) = raw.strip_prefix("file://") {
            let p = PathBuf::from(s);
            if !p.is_absolute()
                || s.contains(['%', '\\', '?', '#'])
                || s.contains("//")
                || s.split('/').any(|part| matches!(part, "." | ".."))
                || p.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
            {
                return Err(fail("invalid file location"));
            }
            return Ok(Self::File(p));
        }
        let u = reqwest::Url::parse(raw).map_err(|_| fail("invalid release URL"))?;
        if u.scheme() != "https"
            || u.host_str().is_none()
            || !u.username().is_empty()
            || u.password().is_some()
            || u.fragment().is_some()
            || u.query().is_some()
            || raw.contains(['%', '\\'])
            || u.path().contains("//")
            || raw.split('/').any(|s| s == "." || s == "..")
        {
            return Err(fail(
                "release URL must be unambiguous HTTPS without credentials, query or fragment",
            ));
        }
        Ok(Self::Https(raw.to_string()))
    }
    pub(super) fn child(&self, s: &str) -> Result<Self> {
        match self {
            Self::Https(v) => Self::parse(&format!("{}/{s}", v.trim_end_matches('/'))),
            Self::File(p) => Ok(Self::File(p.join(s))),
        }
    }
    pub(super) fn pin_digest(&self) -> Option<&str> {
        match self {
            Self::Https(s) => s.split("/pinsets/sha256/").nth(1)?.split('/').next(),
            Self::File(p) => p
                .to_str()?
                .split("/pinsets/sha256/")
                .nth(1)?
                .split('/')
                .next(),
        }
    }
}
