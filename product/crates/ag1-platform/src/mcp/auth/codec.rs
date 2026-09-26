//! Bounded private envelope. Only public headers use JSON; secrets stay in
//! length-prefixed native byte fields and never acquire serde/Debug support.
use super::*;
use crate::mcp::wire::{Limits, decode_json};
const MAGIC: &[u8] = b"AG1-MCP-PRIVATE-1\0";
const MAX_HEADER: usize = 48 * 1024;
struct Writer(Vec<u8>);
impl Drop for Writer {
    fn drop(&mut self) {
        drop(SecretBytes::new(std::mem::take(&mut self.0)));
    }
}
impl Writer {
    fn put(&mut self, bytes: &[u8]) -> Result<(), DomainError> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > ag1_storage::mcp_private::MAX_BYTES)
        {
            return Err(DomainError::LimitExceeded("MCP private vault bytes".into()));
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    fn number(&mut self, n: usize) -> Result<(), DomainError> {
        self.put(&(u32::try_from(n).map_err(|_| invalid("private envelope length"))?).to_le_bytes())
    }
    fn field(&mut self, bytes: &[u8]) -> Result<(), DomainError> {
        self.number(bytes.len())?;
        self.put(bytes)
    }
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DomainError> {
        if n > self.0.len() {
            return Err(invalid("private envelope length"));
        }
        let (v, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(v)
    }
    fn number(&mut self) -> Result<usize, DomainError> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("private envelope length"))?,
        ) as usize)
    }
    fn field(&mut self, max: usize) -> Result<&'a [u8], DomainError> {
        let n = self.number()?;
        if n > max {
            return Err(DomainError::LimitExceeded("MCP private field bytes".into()));
        }
        self.take(n)
    }
    fn secret(&mut self, max: usize) -> Result<SecretBytes, DomainError> {
        Ok(SecretBytes::new(self.field(max)?.to_vec()))
    }
}
fn validate(record: &Record) -> Result<(), DomainError> {
    record.public.binding.validate()?;
    record
        .public
        .grant
        .validate(&record.public.binding.target)?;
    if record.public.generation == 0 {
        return Err(invalid("credential generation"));
    }
    match (&record.public.status, &record.material) {
        (Status::Connected, Some(m)) => m.validate(&record.public.binding.target),
        (Status::Authenticating | Status::Revoked, None)
            if record.public.grant == Grant::default() =>
        {
            Ok(())
        }
        _ => Err(invalid("credential state")),
    }
}
pub(super) fn encode(records: &BTreeMap<String, Record>) -> Result<SecretBytes, DomainError> {
    encode_rows(records.len(), records.iter())
}
pub(super) fn replacing(
    records: &BTreeMap<String, Record>,
    key: &String,
    next: &Record,
) -> Result<SecretBytes, DomainError> {
    let mut rows: BTreeMap<&String, &Record> = records.iter().collect();
    rows.insert(key, next);
    encode_rows(rows.len(), rows.into_iter())
}
fn encode_rows<'a>(
    count: usize,
    records: impl Iterator<Item = (&'a String, &'a Record)>,
) -> Result<SecretBytes, DomainError> {
    if count > MAX_PROFILES {
        return Err(DomainError::LimitExceeded("MCP credential profiles".into()));
    }
    // One fixed safe-ceiling allocation avoids leaving secret copies in buffers
    // abandoned by Vec growth. The final native buffer clears on drop.
    let mut w = Writer(Vec::with_capacity(ag1_storage::mcp_private::MAX_BYTES));
    w.put(MAGIC)?;
    w.number(count)?;
    for (key, r) in records {
        validate(r)?;
        if key != &r.public.binding.key() {
            return Err(invalid("credential key"));
        }
        let header = serde_json::to_vec(&r.public).map_err(|_| invalid("credential header"))?;
        if header.len() > MAX_HEADER {
            return Err(DomainError::LimitExceeded(
                "MCP private header bytes".into(),
            ));
        }
        w.field(&header)?;
        match &r.material {
            None => w.number(0)?,
            Some(Material::Bearer { access, refresh }) => {
                w.number(1)?;
                w.field(access.expose())?;
                w.field(refresh.as_ref().map_or(&[], SecretBytes::expose))?;
            }
            Some(Material::Environment(values)) => {
                w.number(2)?;
                for v in values.values() {
                    w.field(v.expose())?;
                }
            }
        }
    }
    Ok(SecretBytes::new(std::mem::take(&mut w.0)))
}
pub(super) fn decode(bytes: &[u8]) -> Result<BTreeMap<String, Record>, DomainError> {
    if bytes.len() > ag1_storage::mcp_private::MAX_BYTES {
        return Err(DomainError::LimitExceeded("MCP private vault bytes".into()));
    }
    let mut reader = Reader(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(invalid("private vault version"));
    }
    let count = reader.number()?;
    if count > MAX_PROFILES {
        return Err(DomainError::LimitExceeded("MCP credential profiles".into()));
    }
    let mut records = BTreeMap::new();
    for _ in 0..count {
        let header = decode_json(reader.field(MAX_HEADER)?, Limits::new(MAX_HEADER, 8, 1024)?)?;
        let public: PublicRecord =
            serde_json::from_value(header).map_err(|_| invalid("private credential header"))?;
        public.binding.validate()?;
        let material = match reader.number()? {
            0 => None,
            1 => {
                let access = reader.secret(MAX_SECRET_BYTES)?;
                let refresh = reader.secret(MAX_SECRET_BYTES)?;
                Some(Material::Bearer {
                    access,
                    refresh: (!refresh.expose().is_empty()).then_some(refresh),
                })
            }
            2 => {
                let Target::Local {
                    environment_names, ..
                } = &public.binding.target
                else {
                    return Err(invalid("private environment header"));
                };
                let mut values = BTreeMap::new();
                for name in environment_names {
                    values.insert(name.clone(), reader.secret(4096)?);
                }
                Some(Material::Environment(values))
            }
            _ => return Err(invalid("private credential kind")),
        };
        let record = Record {
            public,
            material,
            refresh: None,
        };
        validate(&record)?;
        if records
            .insert(record.public.binding.key(), record)
            .is_some()
        {
            return Err(invalid("duplicate credential profile"));
        }
    }
    if !reader.0.is_empty() {
        return Err(invalid("private envelope trailing bytes"));
    }
    Ok(records)
}
