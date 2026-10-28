//! A backup of Kova's profiles and settings in one file, for moving to another device or
//! keeping a copy. With a password the file is encrypted (PBKDF2-HMAC-SHA256, then
//! AES-256-GCM); without one it is plain JSON, and whoever has it can use the
//! subscriptions in it.

use std::num::NonZeroU32;

use base64::Engine as _;
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use crate::Error;
use crate::store::{Profile, Settings};

const FORMAT: u32 = 1;
const ITERATIONS: u32 = 600_000;

/// What a backup holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    /// The Kova version that made it.
    pub kova: String,
    /// Unix seconds.
    pub created: i64,
    pub settings: Settings,
    pub profiles: Vec<BackupProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupProfile {
    #[serde(flatten)]
    pub profile: Profile,
    /// The profile's content as last downloaded or edited, so it works before the
    /// first update on the new device.
    pub body: String,
}

/// The file: a marker, then the backup or its encrypted form.
#[derive(Serialize, Deserialize)]
struct File {
    kova_backup: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backup: Option<Backup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encrypted: Option<Sealed>,
}

#[derive(Serialize, Deserialize)]
struct Sealed {
    kdf: String,
    iterations: u32,
    salt: String,
    nonce: String,
    data: String,
}

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn key(password: &str, salt: &[u8], iterations: u32) -> Result<LessSafeKey, Error> {
    let rounds = NonZeroU32::new(iterations).ok_or_else(not_a_backup)?;
    let mut k = [0u8; 32];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, rounds, salt, password.as_bytes(), &mut k);
    Ok(LessSafeKey::new(UnboundKey::new(&aead::AES_256_GCM, &k).map_err(|_| Error::Other("could not make the key".into()))?))
}

fn json_error(e: serde_json::Error) -> Error {
    Error::Other(e.to_string())
}

fn not_a_backup() -> Error {
    Error::Other("This is not a Kova backup".into())
}

/// The backup as file text; encrypted when `password` is given and not empty.
pub fn write(b: &Backup, password: Option<&str>) -> Result<String, Error> {
    let file = match password.filter(|p| !p.is_empty()) {
        None => File { kova_backup: FORMAT, backup: Some(b.clone()), encrypted: None },
        Some(password) => {
            let rng = SystemRandom::new();
            let (mut salt, mut nonce) = ([0u8; 16], [0u8; 12]);
            rng.fill(&mut salt).and_then(|_| rng.fill(&mut nonce)).map_err(|_| Error::Other("no random numbers".into()))?;
            let mut data = serde_json::to_vec(b).map_err(json_error)?;
            key(password, &salt, ITERATIONS)?
                .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data)
                .map_err(|_| Error::Other("could not encrypt the backup".into()))?;
            let sealed = Sealed { kdf: "pbkdf2-sha256".into(), iterations: ITERATIONS, salt: B64.encode(salt), nonce: B64.encode(nonce), data: B64.encode(data) };
            File { kova_backup: FORMAT, backup: None, encrypted: Some(sealed) }
        }
    };
    serde_json::to_string_pretty(&file).map_err(json_error)
}

fn parse(text: &str) -> Result<File, Error> {
    let file: File = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| not_a_backup())?;
    if file.kova_backup == 0 || file.kova_backup > FORMAT {
        return Err(Error::Other("This backup is from a newer Kova. Update Kova first".into()));
    }
    Ok(file)
}

/// Whether `text` is a Kova backup that needs its password to be read.
pub fn needs_password(text: &str) -> Result<bool, Error> {
    Ok(parse(text)?.encrypted.is_some())
}

/// The backup in `text`, opened with `password` when it is encrypted.
pub fn read(text: &str, password: Option<&str>) -> Result<Backup, Error> {
    let file = parse(text)?;
    if let Some(b) = file.backup {
        return Ok(b);
    }
    let s = file.encrypted.ok_or_else(not_a_backup)?;
    let password = password.filter(|p| !p.is_empty()).ok_or_else(|| Error::Other("This backup needs its password".into()))?;
    // A file can claim any number of rounds; a sane range keeps a crafted one from
    // tying the app up.
    if s.kdf != "pbkdf2-sha256" || !(10_000..=10_000_000).contains(&s.iterations) {
        return Err(not_a_backup());
    }
    let decode = |v: &str| B64.decode(v).map_err(|_| not_a_backup());
    let (salt, nonce, mut data) = (decode(&s.salt)?, decode(&s.nonce)?, decode(&s.data)?);
    let nonce = Nonce::try_assume_unique_for_key(&nonce).map_err(|_| not_a_backup())?;
    let plain = key(password, &salt, s.iterations)?
        .open_in_place(nonce, Aad::empty(), &mut data)
        .map_err(|_| Error::Other("The password is wrong".into()))?;
    serde_json::from_slice(plain).map_err(|_| not_a_backup())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Source;
    use kova_core::subscription::Format;

    fn sample() -> Backup {
        let profile = Profile {
            id: "abc123".into(),
            name: "Night Owl".into(),
            source: Source::Remote { url: "https://sub.example.com/s/test".into(), user_agent: None, update_hours: 12, via_proxy: true },
            format: Format::Links,
            core: Default::default(),
            node_count: 1,
            usage: None,
            updated_at: 1,
            warnings: vec![],
            support_url: None,
            announce: None,
        };
        Backup { kova: "0.0.0".into(), created: 1, settings: Settings::default(), profiles: vec![BackupProfile { profile, body: "vless://example".into() }] }
    }

    #[test]
    fn plain_backup_reads_back() {
        let text = write(&sample(), None).unwrap();
        assert!(!needs_password(&text).unwrap());
        let b = read(&text, None).unwrap();
        assert_eq!(b.profiles[0].profile.name, "Night Owl");
        assert_eq!(b.profiles[0].body, "vless://example");
    }

    #[test]
    fn encrypted_backup_needs_the_right_password() {
        let text = write(&sample(), Some("correct horse")).unwrap();
        assert!(needs_password(&text).unwrap());
        assert!(!text.contains("sub.example.com"));
        assert!(read(&text, None).unwrap_err().to_string().contains("needs its password"));
        assert!(read(&text, Some("wrong")).unwrap_err().to_string().contains("password is wrong"));
        assert_eq!(read(&text, Some("correct horse")).unwrap().profiles.len(), 1);
    }

    #[test]
    fn other_files_are_not_backups() {
        assert!(needs_password("proxies: []").is_err());
        assert!(read(r#"{"kova_backup": 9}"#, None).unwrap_err().to_string().contains("newer Kova"));
    }
}
