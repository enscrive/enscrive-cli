//! Bounded two-pass tar admission. Raw metadata is inspected before tar can read_all it.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
const MAX_ENTRIES: usize = 100_000;
const MAX_EXPANDED: u64 = 8 * 1024 * 1024 * 1024;
const MAX_METADATA: u64 = 1024 * 1024;
fn open(path: &Path) -> Result<tar::Archive<flate2::read::GzDecoder<File>>> {
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(
        File::open(path).map_err(|_| fail("archive unavailable"))?,
    )))
}
fn raw_admission(path: &Path, deadline: Instant) -> Result<()> {
    raw_admission_limits(path, deadline, MAX_ENTRIES, MAX_EXPANDED, MAX_COMPONENT)
}
/// One complete gzip member, with all decoded framing/padding counted.
fn raw_admission_limits(
    path: &Path,
    deadline: Instant,
    entries_limit: usize,
    expanded_limit: u64,
    member_limit: u64,
) -> Result<()> {
    use std::io::{BufRead, BufReader};
    let compressed = BufReader::new(File::open(path).map_err(|_| fail("archive unavailable"))?);
    // bufread decoder leaves read-ahead bytes in its returned BufReader.
    let mut decoder = flate2::bufread::GzDecoder::new(compressed);
    let mut total = 0u64;
    let mut count = 0usize;
    loop {
        let mut block = [0u8; 512];
        framed_exact(
            &mut decoder,
            &mut block,
            &mut total,
            expanded_limit,
            deadline,
        )?;
        if block.iter().all(|b| *b == 0) {
            framed_exact(
                &mut decoder,
                &mut block,
                &mut total,
                expanded_limit,
                deadline,
            )?;
            if block.iter().any(|b| *b != 0) {
                return Err(fail("archive two-block terminator missing"));
            }
            let mut padding = [0u8; 65536];
            loop {
                let n = framed_read(
                    &mut decoder,
                    &mut padding,
                    &mut total,
                    expanded_limit,
                    deadline,
                )?;
                if n == 0 {
                    break;
                }
                if padding[..n].iter().any(|b| *b != 0) {
                    return Err(fail("archive nonzero trailing data"));
                }
            }
            // Reading decoded EOF validates CRC/length/trailer. The bufread wrapper
            // preserves compressed lookahead, so this tests actual compressed EOF.
            let mut remaining = decoder.into_inner();
            check_time(deadline)?;
            if !remaining
                .fill_buf()
                .map_err(|_| fail("archive compressed EOF unavailable"))?
                .is_empty()
            {
                return Err(fail("archive extra compressed member or suffix"));
            }
            return check_time(deadline);
        }
        count += 1;
        if count > entries_limit {
            return Err(fail("archive entry limit"));
        }
        let header = tar::Header::from_byte_slice(&block);
        let checksum = block
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    u32::from(*b)
                }
            })
            .sum::<u32>();
        if header
            .cksum()
            .map_err(|_| fail("archive checksum invalid"))?
            != checksum
        {
            return Err(fail("archive checksum invalid"));
        }
        let kind = header.entry_type();
        let size = header
            .size()
            .map_err(|_| fail("archive member size invalid"))?;
        let metadata = kind.is_gnu_longname() || kind.is_pax_local_extensions();
        // These non-installed metadata headers may omit exactly all eight mode bytes.
        // Every other representation, including malformed real entries, remains checked.
        let omitted_metadata_mode = metadata && header.as_old().mode.iter().all(|b| *b == 0);
        if !omitted_metadata_mode
            && header.mode().map_err(|_| fail("archive mode invalid"))? & 0o7000 != 0
        {
            return Err(fail("archive privilege bits refused"));
        }
        if metadata {
            if size > MAX_METADATA {
                return Err(fail("archive metadata limit"));
            }
        } else if kind.is_file() || kind.is_dir() {
            if size > member_limit || kind.is_dir() && size != 0 {
                return Err(fail("archive member size invalid"));
            }
        } else {
            return Err(fail("unsupported archive entry or extension"));
        }
        let mut stored = Vec::new();
        let mut left = size;
        let mut buffer = [0u8; 65536];
        while left > 0 {
            let len = usize::try_from(left.min(buffer.len() as u64)).expect("bounded buffer");
            framed_exact(
                &mut decoder,
                &mut buffer[..len],
                &mut total,
                expanded_limit,
                deadline,
            )?;
            if metadata {
                stored.extend_from_slice(&buffer[..len]);
            }
            left -= len as u64;
        }
        if metadata {
            if kind.is_gnu_longname() {
                if stored.last() == Some(&0) {
                    stored.pop();
                }
                normalize(&stored)?;
            } else {
                validate_pax(&stored)?;
            }
        }
        let padding = ((512 - size % 512) % 512) as usize;
        if padding > 0 {
            framed_exact(
                &mut decoder,
                &mut buffer[..padding],
                &mut total,
                expanded_limit,
                deadline,
            )?;
            if buffer[..padding].iter().any(|b| *b != 0) {
                return Err(fail("archive nonzero member padding"));
            }
        }
    }
}
fn framed_read(
    input: &mut dyn Read,
    buffer: &mut [u8],
    total: &mut u64,
    limit: u64,
    deadline: Instant,
) -> Result<usize> {
    check_time(deadline)?;
    let n = input
        .read(buffer)
        .map_err(|_| fail("archive gzip/framing invalid"))?;
    *total = total
        .checked_add(n as u64)
        .ok_or_else(|| fail("archive expansion limit"))?;
    if *total > limit {
        return Err(fail("archive expansion limit"));
    }
    check_time(deadline)?;
    Ok(n)
}
fn framed_exact(
    input: &mut dyn Read,
    mut buffer: &mut [u8],
    total: &mut u64,
    limit: u64,
    deadline: Instant,
) -> Result<()> {
    while !buffer.is_empty() {
        let n = framed_read(input, buffer, total, limit, deadline)?;
        if n == 0 {
            return Err(fail("archive truncated framing or terminator"));
        }
        buffer = &mut buffer[n..];
    }
    Ok(())
}
fn validate_pax(data: &[u8]) -> Result<()> {
    let mut seen = BTreeSet::new();
    // Parse exact length-prefixed PAX records, including their newline, without permissive fallback.
    let mut at = 0usize;
    while at < data.len() {
        let space = data[at..]
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| fail("PAX framing invalid"))?
            + at;
        let digits =
            std::str::from_utf8(&data[at..space]).map_err(|_| fail("PAX framing invalid"))?;
        if digits.is_empty() || !digits.bytes().all(|x| x.is_ascii_digit()) {
            return Err(fail("PAX framing invalid"));
        }
        let length: usize = digits.parse().map_err(|_| fail("PAX framing invalid"))?;
        let end = at
            .checked_add(length)
            .ok_or_else(|| fail("PAX framing invalid"))?;
        if end > data.len() || end <= space + 2 || data[end - 1] != b'\n' {
            return Err(fail("PAX framing invalid"));
        }
        let row = &data[space + 1..end - 1];
        let eq = row
            .iter()
            .position(|b| *b == b'=')
            .ok_or_else(|| fail("PAX framing invalid"))?;
        let key = std::str::from_utf8(&row[..eq]).map_err(|_| fail("PAX key invalid"))?;
        let value = &row[eq + 1..];
        if !seen.insert(key) {
            return Err(fail("duplicate PAX key"));
        }
        match key {
            "path" => {
                normalize(value)?;
            }
            // Ownership and timestamps are deliberately not restored. Size overrides can desynchronize
            // raw framing and effective extraction; refuse them rather than trust two different parsers.
            "mtime" | "atime" | "ctime" | "uid" | "gid" | "uname" | "gname" => {
                if value.contains(&0) {
                    return Err(fail("PAX metadata invalid"));
                }
            }
            _ => return Err(fail("unsupported PAX extension")),
        }
        at = end;
    }
    Ok(())
}
fn normalize(bytes: &[u8]) -> Result<PathBuf> {
    let text = std::str::from_utf8(bytes).map_err(|_| fail("archive path must be UTF-8"))?;
    if text.len() > 4096 || text.starts_with('/') || text.contains(['\\', ':', '\0']) {
        return Err(fail("archive path invalid"));
    }
    let mut out = PathBuf::new();
    for p in text.split('/') {
        match p {
            "" | "." => {}
            ".." => return Err(fail("archive traversal refused")),
            _ => out.push(p),
        }
    }
    Ok(out)
}
fn bounded_copy(
    input: &mut dyn Read,
    output: &mut dyn Write,
    expected: u64,
    deadline: Instant,
) -> Result<()> {
    let mut total = 0u64;
    let mut b = [0; 65536];
    loop {
        check_time(deadline)?;
        let n = input
            .read(&mut b)
            .map_err(|_| fail("archive data invalid"))?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or_else(|| fail("archive size overflow"))?;
        if total > expected {
            return Err(fail("archive declared size mismatch"));
        }
        output
            .write_all(&b[..n])
            .map_err(|_| fail("archive output failed"))?;
    }
    if total != expected {
        return Err(fail("archive declared size mismatch"));
    }
    check_time(deadline)
}
pub(super) fn extract(
    input: &Path,
    root: &Path,
    expected: &str,
    deadline: Instant,
) -> Result<Vec<serde_json::Value>> {
    raw_admission(input, deadline)?;
    let mut tar = open(input)?;
    let mut explicit = BTreeSet::new();
    let mut kinds: BTreeMap<PathBuf, bool> = BTreeMap::new();
    let mut total = 0u64;
    let mut count = 0usize;
    for e in tar.entries().map_err(|_| fail("archive invalid"))? {
        check_time(deadline)?;
        let mut e = e.map_err(|_| fail("archive invalid"))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(fail("archive entry limit"));
        }
        let p = normalize(e.path_bytes().as_ref())?;
        let dir = e.header().entry_type().is_dir();
        if !dir && !e.header().entry_type().is_file() {
            return Err(fail("archive effective type invalid"));
        }
        if !explicit.insert(p.clone()) {
            return Err(fail("duplicate archive path"));
        }
        if p.as_os_str().is_empty() {
            if !dir {
                return Err(fail("archive root entry invalid"));
            }
            continue;
        }
        if let Some(was_dir) = kinds.get(&p)
            && (!*was_dir || !dir)
        {
            return Err(fail("archive type collision"));
        }
        for ancestor in p.ancestors().skip(1).filter(|p| !p.as_os_str().is_empty()) {
            if kinds.get(ancestor) == Some(&false) {
                return Err(fail("archive type collision"));
            }
            kinds.insert(ancestor.to_path_buf(), true);
        }
        kinds.insert(p.clone(), dir);
        if kinds.len() > MAX_ENTRIES {
            return Err(fail("archive effective path limit"));
        }
        let dest = root.join(&p);
        if let Some(parent) = dest.parent() {
            private_dir(parent)?;
        }
        if dir {
            private_dir(&dest)?;
        } else {
            let size = e.size();
            total = total
                .checked_add(size)
                .ok_or_else(|| fail("archive expansion limit"))?;
            if size > MAX_COMPONENT || total > MAX_EXPANDED {
                return Err(fail("archive expansion limit"));
            }
            let mut f = new_file(&dest)?;
            bounded_copy(&mut e, &mut f, size, deadline)?;
            f.sync_all().map_err(|_| fail("archive sync failed"))?;
        }
    }
    let binary = root.join(expected);
    let m =
        fs::symlink_metadata(&binary).map_err(|_| fail("archive expected executable missing"))?;
    if !m.is_file() || m.len() == 0 {
        return Err(fail("archive expected executable invalid"));
    }
    executable(&binary)?;
    // Enumerate the complete accepted tree, including implicitly created directories.
    let mut inventory = Vec::new();
    enumerate(root, root, &mut inventory, deadline)?;
    sync_dir(root)?;
    check_time(deadline)?;
    Ok(inventory)
}
fn enumerate(
    root: &Path,
    at: &Path,
    items: &mut Vec<serde_json::Value>,
    deadline: Instant,
) -> Result<()> {
    // Iterative traversal avoids making adversarial directory depth a call-stack bound.
    let mut pending = vec![at.to_path_buf()];
    while let Some(directory) = pending.pop() {
        check_time(deadline)?;
        let mut entries = fs::read_dir(&directory)
            .map_err(|_| fail("archive inventory unavailable"))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| fail("archive inventory unavailable"))?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            check_time(deadline)?;
            if items.len() >= MAX_ENTRIES {
                return Err(fail("archive inventory entry limit"));
            }
            let p = e.path();
            let rel = p
                .strip_prefix(root)
                .map_err(|_| fail("archive inventory path invalid"))?;
            let m = fs::symlink_metadata(&p).map_err(|_| fail("archive inventory unavailable"))?;
            if m.is_dir() {
                items.push(serde_json::json!({"path":rel,"type":"directory"}));
                pending.push(p);
            } else if m.is_file() {
                let id = identity(&p, deadline, MAX_COMPONENT)?;
                items.push(serde_json::json!({"path":rel,"type":"file","bytes":id.size,"sha256":id.sha256}));
            } else {
                return Err(fail("archive inventory unexpected type"));
            }
        }
        sync_dir(&directory)?;
    }
    items.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ens5913_archive_admission_limits_and_metadata() {
        let t = tempfile::TempDir::new().unwrap();
        let p = t.path().join("input.tgz");
        super::super::tests::archive_fixture(
            &p,
            &[
                ("a", tar::EntryType::Regular, b"1234"),
                ("b", tar::EntryType::Regular, b"5678"),
            ],
        );
        let d = Instant::now() + Duration::from_secs(2);
        assert!(raw_admission_limits(&p, d, 2, 3072, 4).is_ok());
        assert!(raw_admission_limits(&p, d, 1, 3072, 4).is_err());
        assert!(raw_admission_limits(&p, d, 2, 3071, 4).is_err());
        assert!(raw_admission_limits(&p, d, 2, 3072, 3).is_err());
        for path in ["../x", "/absolute", "C:/drive", "a/../b", "a\\b"] {
            assert!(normalize(path.as_bytes()).is_err());
        }
        assert!(normalize("a".repeat(4097).as_bytes()).is_err());
        assert_eq!(
            normalize(b"./site/pkg/app.js").unwrap(),
            PathBuf::from("site/pkg/app.js")
        );
        assert!(validate_pax(b"14 path=a.txt\n").is_ok());
        assert!(validate_pax(b"15 path=a.txt\n").is_err()); // incorrect length must not be tolerated
        assert!(validate_pax(b"12 size=100\n").is_err()); // even well-framed size overrides are unsupported
    }
    #[test]
    fn ens5913_archive_missing_executable_and_file_directory_collision() {
        let t = tempfile::TempDir::new().unwrap();
        for (n, entries) in [
            vec![("site/pkg/app.js", tar::EntryType::Regular, &b"asset"[..])],
            vec![
                ("a", tar::EntryType::Regular, &b"file"[..]),
                ("a/b", tar::EntryType::Regular, &b"child"[..]),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            let p = t.path().join(format!("{n}.tgz"));
            super::super::tests::archive_fixture(&p, &entries);
            let out = t.path().join(format!("out{n}"));
            private_dir(&out).unwrap();
            assert!(
                extract(
                    &p,
                    &out,
                    "enscrive-developer",
                    Instant::now() + Duration::from_secs(2)
                )
                .is_err()
            );
        }
    }
}

#[cfg(test)]
mod framing_tests {
    use super::*;
    fn compressed(bytes: &[u8]) -> Vec<u8> {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(bytes).unwrap();
        gzip.finish().unwrap()
    }
    fn ordinary() -> Vec<u8> {
        let mut h = tar::Header::new_gnu();
        h.set_path("enscrive-developer").unwrap();
        h.set_mode(0o755);
        h.set_size(3);
        h.set_cksum();
        let mut raw = h.as_bytes().to_vec();
        raw.extend_from_slice(b"exe");
        raw.resize(2048, 0);
        raw
    }
    #[test]
    fn ens5913_metadata_mode_exact_omission_and_type_sensitivity() {
        let t = tempfile::TempDir::new().unwrap();
        let p = t.path().join("mode.tgz");
        let mut case = 0;
        let mut run = |kind: tar::EntryType, mode: [u8; 8], expected: Option<&str>| {
            let metadata = kind.is_gnu_longname() || kind.is_pax_local_extensions();
            let body: &[u8] = if kind.is_gnu_longname() {
                b"enscrive-developer\0"
            } else if kind.is_pax_local_extensions() {
                b"27 path=enscrive-developer\n"
            } else if kind.is_file() {
                b"exe"
            } else {
                b""
            };
            let mut h = tar::Header::new_gnu();
            h.set_path(if metadata { "metadata" } else { "enscrive-developer" })
                .unwrap();
            h.set_entry_type(kind);
            h.set_size(body.len() as u64);
            if kind == tar::EntryType::GNUSparse {
                h.as_gnu_mut().unwrap().set_real_size(body.len() as u64);
            }
            h.as_old_mut().mode = mode;
            h.set_cksum(); // Every negative reaches mode/type admission, not checksum refusal.
            let mut raw = h.as_bytes().to_vec();
            raw.extend_from_slice(body);
            raw.resize(raw.len().div_ceil(512) * 512, 0);
            if metadata {
                raw.extend_from_slice(&ordinary());
            } else {
                raw.resize(raw.len() + 1024, 0);
            }
            fs::write(&p, compressed(&raw)).unwrap();
            let out = t.path().join(format!("out-{case}"));
            case += 1;
            private_dir(&out).unwrap();
            let result = extract(
                &p,
                &out,
                "enscrive-developer",
                Instant::now() + Duration::from_secs(2),
            );
            if let Some(error) = expected {
                assert_eq!(result.unwrap_err(), fail(error), "kind={kind:?} mode={mode:?}");
                assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
            } else {
                result.unwrap();
                assert_eq!(fs::read(out.join("enscrive-developer")).unwrap(), b"exe");
                assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
            }
        };
        for kind in [tar::EntryType::Regular, tar::EntryType::Directory] {
            for mode in [[0; 8], *b"notmode!", [b' '; 8], [0, 32, 0, 0, 0, 0, 0, 0]] {
                run(kind, mode, Some("archive mode invalid"));
            }
            run(kind, *b"0004755\0", Some("archive privilege bits refused"));
        }
        for kind in [tar::EntryType::GNULongName, tar::EntryType::XHeader] {
            for mode in [*b"notmode!", [b' '; 8], [0, 32, 0, 0, 0, 0, 0, 0]] {
                run(kind, mode, Some("archive mode invalid"));
            }
            run(kind, *b"0004755\0", Some("archive privilege bits refused"));
            for mode in [[0; 8], *b"0000644\0", *b"0000000\0"] {
                run(kind, mode, None);
            }
        }
        for kind in [tar::EntryType::XGlobalHeader, tar::EntryType::Link, tar::EntryType::GNUSparse] {
            run(kind, [0; 8], Some("archive mode invalid"));
            run(kind, *b"0000644\0", Some("unsupported archive entry or extension"));
        }
    }
    #[test]
    fn ens5913_complete_single_gzip_tar_framing() {
        let t = tempfile::TempDir::new().unwrap();
        let p = t.path().join("archive.tgz");
        let raw = ordinary();
        let valid = compressed(&raw);
        fs::write(&p, &valid).unwrap();
        let d = Instant::now() + Duration::from_secs(2);
        assert!(raw_admission(&p, d).is_ok());
        let mut padded = raw.clone();
        padded.resize(10240, 0);
        fs::write(&p, compressed(&padded)).unwrap();
        assert!(raw_admission(&p, d).is_ok());
        assert_eq!(
            raw_admission_limits(&p, d, 10, 10239, 100).unwrap_err(),
            fail("archive expansion limit")
        );
        let mut corrupt = valid.clone();
        let i = corrupt.len() - 8;
        corrupt[i] ^= 1;
        let mut extra = valid.clone();
        extra.extend_from_slice(&compressed(&raw));
        let mut suffix = valid.clone();
        suffix.push(1);
        for (label, bytes, expected) in [
            ("crc", corrupt, "archive gzip/framing invalid"),
            (
                "trailer",
                valid[..valid.len() - 1].to_vec(),
                "archive gzip/framing invalid",
            ),
            ("second", extra, "archive extra compressed member or suffix"),
            (
                "suffix",
                suffix,
                "archive extra compressed member or suffix",
            ),
            (
                "no-terminator",
                compressed(&raw[..1024]),
                "archive truncated framing or terminator",
            ),
            (
                "one-zero",
                compressed(&raw[..1536]),
                "archive truncated framing or terminator",
            ),
        ] {
            fs::write(&p, bytes).unwrap();
            assert_eq!(raw_admission(&p, d).unwrap_err(), fail(expected), "{label}");
        }
        let mut nonzero = raw.clone();
        nonzero.push(1);
        fs::write(&p, compressed(&nonzero)).unwrap();
        assert_eq!(
            raw_admission(&p, d).unwrap_err(),
            fail("archive nonzero trailing data")
        );
        let mut bad_padding = raw;
        bad_padding[515] = 1;
        fs::write(&p, compressed(&bad_padding)).unwrap();
        assert_eq!(
            raw_admission(&p, d).unwrap_err(),
            fail("archive nonzero member padding")
        );
    }
}
