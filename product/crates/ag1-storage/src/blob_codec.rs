//! Physical custody format v1: AG1ZLIB1, little-endian logical u64 length,
//! exactly one zlib stream. Logical addresses always hash the decoded bytes.
use crate::persistence::{no_symlink, valid_digest};
use crate::{StorageError, StorageResult, Store, digest};
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
};

const MAGIC: &[u8; 8] = b"AG1ZLIB1";
const MAX: u64 = 256 * 1024 * 1024;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Decoder<R> {
    source: BufReader<R>,
    decoder: Decompress,
    length: u64,
    done: bool,
}
impl<R: Read> Decoder<R> {
    fn new(mut source: R) -> io::Result<Self> {
        let mut header = [0; 16];
        source.read_exact(&mut header)?;
        let length = u64::from_le_bytes(header[8..].try_into().unwrap());
        if &header[..8] != MAGIC || length > MAX {
            return Err(invalid("Unsupported or oversized compressed blob"));
        }
        Ok(Self {
            source: BufReader::with_capacity(65536, source),
            decoder: Decompress::new(true),
            length,
            done: false,
        })
    }
}
impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        loop {
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            // At most one excess byte is decoded before rejecting a false length.
            let cap = output.len().min((self.length - before_out + 1) as usize);
            let input = self.source.fill_buf()?;
            let status = self
                .decoder
                .decompress(input, &mut output[..cap], FlushDecompress::None)
                .map_err(|_| invalid("Corrupt compressed blob"))?;
            let consumed = (self.decoder.total_in() - before_in) as usize;
            let produced = (self.decoder.total_out() - before_out) as usize;
            self.source.consume(consumed);
            if self.decoder.total_out() > self.length {
                return Err(invalid("Compressed blob length exceeded"));
            }
            if status == Status::StreamEnd {
                if self.decoder.total_out() != self.length || !self.source.fill_buf()?.is_empty() {
                    return Err(invalid("Compressed blob length mismatch or trailing bytes"));
                }
                self.done = true;
            } else if produced == 0 && consumed == 0 {
                return Err(invalid("Truncated compressed blob"));
            }
            if produced != 0 || self.done {
                return Ok(produced);
            }
        }
    }
}

pub(crate) fn physical_hash(name: &str) -> Option<&str> {
    let hash = name.strip_suffix(".z1").unwrap_or(name);
    valid_digest(hash).then_some(hash)
}

impl Store {
    pub(crate) fn physical_blob(&self, hash: &str) -> StorageResult<(PathBuf, bool)> {
        if !valid_digest(hash) {
            return Err(StorageError::Invalid("Blob digest".into()));
        }
        let raw = self.root.join("blobs").join(hash);
        // Deterministic raw preference; never hide a corrupt selected representation.
        match std::fs::symlink_metadata(&raw) {
            Ok(_) => Ok((raw, false)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((raw.with_extension("z1"), true)),
            Err(e) => Err(e.into()),
        }
    }
    pub(crate) fn blob_reader(&self, hash: &str) -> StorageResult<Box<dyn Read>> {
        let (path, compressed) = self.physical_blob(hash)?;
        no_symlink(&path)?;
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(StorageError::Corrupt("Non-file blob".into()));
        }
        if compressed && file.metadata()?.len() > MAX {
            return Err(StorageError::Limit(
                "Compressed blob physical size exceeds 256 MiB".into(),
            ));
        }
        if compressed {
            Ok(Box::new(Decoder::new(file)?))
        } else {
            Ok(Box::new(file))
        }
    }
    /// Export the selected physical representation, validating those same bytes.
    pub(crate) fn export_blob(&self, hash: &str) -> StorageResult<(String, Vec<u8>)> {
        let (path, compressed) = self.physical_blob(hash)?;
        no_symlink(&path)?;
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(StorageError::Corrupt("Non-file blob".into()));
        }
        let mut physical = Vec::new();
        file.take(MAX + 1).read_to_end(&mut physical)?;
        if physical.len() as u64 > MAX {
            return Err(StorageError::Limit("Blob export exceeds 256 MiB".into()));
        }
        use sha2::{Digest, Sha256};
        let mut reader: Box<dyn Read + '_> = if compressed {
            Box::new(Decoder::new(physical.as_slice())?)
        } else {
            Box::new(physical.as_slice())
        };
        let mut hasher = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        if format!("{:x}", hasher.finalize()) != hash {
            return Err(StorageError::Corrupt("Export blob hash mismatch".into()));
        }
        drop(reader);
        Ok((
            format!("{hash}{}", if compressed { ".z1" } else { "" }),
            physical,
        ))
    }
    /// Only large, measured custody kinds opt in. The caller activates schema 2
    /// in the same transaction that publishes the first compressed reference.
    pub(crate) fn publish_custody(
        &self,
        bytes: &[u8],
        kind: &str,
    ) -> StorageResult<(String, bool)> {
        if bytes.len() < 16 * 1024
            || bytes.len() as u64 > MAX
            || !matches!(
                kind,
                "context-original-journal" | "runtime-request-input" | "context-original-request"
            )
        {
            return self.publish(bytes).map(|hash| (hash, false));
        }
        let hash = digest(bytes);
        let (path, compressed) = self.physical_blob(&hash)?;
        if path.try_exists()? {
            self.verify_blob(&hash)?;
            return Ok((hash, compressed));
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes)?;
        let encoded = encoder.finish()?;
        if encoded.len() + 16 > bytes.len() - bytes.len() / 8 {
            return self.publish(bytes).map(|hash| (hash, false));
        }
        let mut physical = Vec::with_capacity(encoded.len() + 16);
        physical.extend_from_slice(MAGIC);
        physical.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        physical.extend_from_slice(&encoded);
        self.publish_physical(&path, &physical)?;
        Ok((hash, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactInput, Boundary, FaultInjector};
    use ag1_core::{Clock, DomainError, SessionId, evidence::EvidenceRequest};
    use serde_json::json;
    use std::{fs, sync::Arc};

    fn seed(store: &Store) {
        store.create_session("session", &serde_json::to_vec(&json!({"type":"session","version":3,"id":"session","cwd":"/fixture","timestamp":"2026-09-13T00:00:00Z"})).unwrap()).unwrap();
        store
            .register_origin("origin", "session", &[7; 32])
            .unwrap();
    }
    fn input(kind: &str) -> ArtifactInput {
        let mut bytes = b"custody row\n".repeat(100_000);
        bytes[65530..65544].copy_from_slice(b"boundaryneedle");
        let record = json!({"version":1,"id":"art_00000000000000000000000000000001","sessionId":"session","kind":kind,"mimeType":"text/plain","encoding":"utf8","createdAt":"2026-09-13T00:00:00Z","sha256":digest(&bytes),"bytes":bytes.len(),"producer":{"type":"system","name":"fixture"},"unknown":{"retained":true}});
        ArtifactInput {
            origin: "origin".into(),
            session: "session".into(),
            record: serde_json::to_vec(&record).unwrap(),
            bytes,
        }
    }
    fn request() -> EvidenceRequest {
        EvidenceRequest {
            requester: SessionId::new("session").unwrap(),
            origin: "origin".into(),
            artifact: "art_00000000000000000000000000000001".into(),
        }
    }
    #[test]
    fn compressed_roundtrip_windows_search_cancellation_gc_and_exact_export() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store");
        let store = Store::open(&path).unwrap();
        seed(&store);
        assert_eq!(store.inspect_integrity().unwrap().schema, 1);
        let input = input("context-original-journal");
        let hash = digest(&input.bytes);
        let id = store.put_artifact(&input).unwrap();
        assert_eq!(store.inspect_integrity().unwrap().schema, 2);
        store
            .conn()
            .unwrap()
            .execute_batch("PRAGMA user_version=1;")
            .unwrap();
        assert!(
            store.inspect_integrity().is_err(),
            "v1 cannot contain a compressed reference"
        );
        store
            .conn()
            .unwrap()
            .execute_batch("PRAGMA user_version=2;")
            .unwrap();

        assert!(matches!(
            store.blob_path(&hash),
            Err(StorageError::Unsupported(_))
        ));
        let physical = path.join("blobs").join(format!("{hash}.z1"));
        let encoded = fs::read(&physical).unwrap();
        assert!(encoded.len() < input.bytes.len() / 10);
        assert_eq!(store.blob_bytes(&hash).unwrap(), input.bytes);
        for offset in [
            0,
            65530,
            65535,
            65536,
            input.bytes.len() - 1,
            input.bytes.len(),
        ] {
            let (size, bytes) = store.blob_window(&hash, offset as u64, 30).unwrap();
            assert_eq!(size, input.bytes.len() as u64);
            assert_eq!(
                bytes,
                input.bytes[offset..(offset + 30).min(input.bytes.len())]
            );
        }
        assert_eq!(
            store.artifact_record("session", "origin", &id).unwrap(),
            input.record
        );
        assert!(store.page("foreign", "origin", &id, 0, 10).is_err());
        let scan = store
            .scan_artifact(&request(), "boundaryneedle", false, 3, || Ok(()))
            .unwrap();
        assert_eq!(scan.total_matches, 1);
        assert_eq!(scan.sha256, hash);
        let mut checks = 0;
        assert!(
            store
                .scan_artifact(&request(), "absent", false, 3, || {
                    checks += 1;
                    if checks > 4 {
                        Err(DomainError::Conflict)
                    } else {
                        Ok(())
                    }
                })
                .is_err()
        );
        assert_eq!(checks, 5);
        let orphan = "a".repeat(64);
        fs::copy(&physical, path.join("blobs").join(format!("{orphan}.z1"))).unwrap();
        assert_eq!(store.inspect_integrity().unwrap().orphan_blobs, 1);
        assert_eq!(store.collect_orphans().unwrap(), 1);
        assert!(physical.exists());
        let export = temp.path().join("export");
        assert_eq!(store.export(&export, true).unwrap()["schema"], 2);
        assert_eq!(
            fs::read(export.join("blobs").join(format!("{hash}.z1"))).unwrap(),
            encoded
        );
        let restored = temp.path().join("restored");
        Store::restore_export(&export, &restored).unwrap();
        let restored = Store::open(&restored).unwrap();
        assert_eq!(restored.blob_bytes(&hash).unwrap(), input.bytes);
        assert_eq!(
            restored.artifact_record("session", "origin", &id).unwrap(),
            input.record
        );
        assert_eq!(restored.inspect_integrity().unwrap().schema, 2);
        drop(store);
        assert_eq!(
            Store::inspect_quiescent(&path, "session").unwrap().0.schema,
            2
        );
        let store = Store::open(&path).unwrap();
        assert_eq!(store.blob_bytes(&hash).unwrap(), input.bytes);
        // Immutable re-publication neither adds a raw copy nor changes physical bytes.
        store.put_artifact(&input).unwrap();
        assert_eq!(fs::read(physical).unwrap(), encoded);
    }
    #[test]
    fn raw_v1_existing_bytes_and_non_custody_publications_stay_raw() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store");
        let store = Store::open(&path).unwrap();
        seed(&store);
        let input = input("text");
        let hash = digest(&input.bytes);
        store.put_artifact(&input).unwrap();
        let raw = store.blob_path(&hash).unwrap();
        let before = fs::read(&raw).unwrap();
        assert_eq!(store.inspect_integrity().unwrap().schema, 1);
        assert_eq!(
            store
                .publish_custody(&input.bytes, "runtime-request-input")
                .unwrap(),
            (hash.clone(), false)
        );
        assert_eq!(fs::read(raw).unwrap(), before);
        assert_eq!(store.blob_bytes(&hash).unwrap(), before);
        let export = temp.path().join("export");
        assert_eq!(store.export(&export, true).unwrap()["schema"], 1);
        Store::restore_export(&export, &temp.path().join("restored")).unwrap();
        assert_eq!(
            store
                .publish_custody(b"small", "context-original-request")
                .unwrap()
                .1,
            false
        );
        let noisy: Vec<u8> = (0..4096u64)
            .flat_map(|n| {
                use sha2::{Digest, Sha256};
                Sha256::digest(n.to_le_bytes()).to_vec()
            })
            .collect();
        assert!(
            !store
                .publish_custody(&noisy, "runtime-request-input")
                .unwrap()
                .1
        );
    }
    #[test]
    fn malformed_compressed_data_never_produces_a_trusted_window_search_or_export() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("store");
        let store = Store::open(&path).unwrap();
        seed(&store);
        let input = input("context-original-request");
        let hash = digest(&input.bytes);
        store.put_artifact(&input).unwrap();
        let file = path.join("blobs").join(format!("{hash}.z1"));
        let original = fs::read(&file).unwrap();
        let mut cases = vec![
            original[..8].to_vec(),
            original[..original.len() - 1].to_vec(),
            original[..original.len() / 2].to_vec(),
        ];
        for (index, value) in [
            (0, b'X'),
            (8, 0),
            (original.len() - 1, original[original.len() - 1] ^ 1),
        ] {
            let mut bad = original.clone();
            bad[index] = value;
            cases.push(bad);
        }
        let mut oversized = original.clone();
        oversized[8..16].copy_from_slice(&(MAX + 1).to_le_bytes());
        cases.push(oversized);
        let mut trailing = original.clone();
        trailing.push(0);
        cases.push(trailing);
        let mut concatenated = original.clone();
        concatenated.extend_from_slice(&original[16..]);
        cases.push(concatenated);
        for (i, bytes) in cases.iter().enumerate() {
            fs::write(&file, bytes).unwrap();
            assert!(store.blob_window(&hash, 0, 10).is_err(), "case {i}");
            assert!(
                store
                    .scan_artifact(&request(), "custody", false, 1, || Ok(()))
                    .is_err(),
                "case {i}"
            );
            assert!(store.export_blob(&hash).is_err(), "case {i}");
        }
        fs::write(&file, &original).unwrap();
        fs::write(
            path.join("blobs").join(&hash),
            b"corrupt preferred raw bytes",
        )
        .unwrap();
        assert!(store.blob_bytes(&hash).is_err());
        fs::remove_file(path.join("blobs").join(&hash)).unwrap();
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(temp.path().join("missing"), &file).unwrap();
        assert!(store.blob_bytes(&hash).is_err());
    }
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            1
        }
    }
    struct Crash(String);
    impl FaultInjector for Crash {
        fn check(&self, boundary: Boundary) -> StorageResult<()> {
            if format!("{boundary:?}") == self.0 {
                std::process::exit(91);
            }
            Ok(())
        }
    }
    #[test]
    #[ignore = "spawned by compressed_publication_crash_recovery"]
    fn crash_child() {
        let path = PathBuf::from(std::env::var_os("AG1_CODEC_CRASH_PATH").unwrap());
        let store = Store::open_with(
            &path,
            Arc::new(FixedClock),
            Arc::new(Crash(std::env::var("AG1_CODEC_CRASH_BOUNDARY").unwrap())),
        )
        .unwrap();
        store.put_artifact(&input("runtime-request-input")).unwrap();
    }
    #[test]
    fn compressed_publication_crash_recovery_is_atomic_at_every_boundary() {
        for boundary in [
            "BeforeBlobSync",
            "AfterBlobSync",
            "BeforeRename",
            "AfterRename",
            "BeforeTransaction",
            "BeforeCommit",
            "AfterCommit",
            "BeforeAck",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("store");
            let store = Store::open(&path).unwrap();
            seed(&store);
            let sequence = store.sequence().unwrap();
            drop(store);
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "blob_codec::tests::crash_child", "--ignored"])
                .env("AG1_CODEC_CRASH_PATH", &path)
                .env("AG1_CODEC_CRASH_BOUNDARY", boundary)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(91), "{boundary}");
            let store = Store::open(&path).unwrap();
            let committed = matches!(boundary, "AfterCommit" | "BeforeAck");
            assert_eq!(
                store.sequence().unwrap(),
                sequence + u64::from(committed),
                "{boundary}"
            );
            assert_eq!(
                store.inspect_integrity().unwrap().schema,
                if committed { 2 } else { 1 },
                "{boundary}"
            );
            let input = input("runtime-request-input");
            assert_eq!(
                store
                    .artifact_record("session", "origin", &request().artifact)
                    .is_ok(),
                committed
            );
            store.collect_orphans().unwrap();
            store.put_artifact(&input).unwrap();
            assert_eq!(
                store.blob_bytes(&digest(&input.bytes)).unwrap(),
                input.bytes
            );
            assert_eq!(store.inspect_integrity().unwrap().schema, 2);
        }
    }
}
