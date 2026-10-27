//! Subscription addresses and profile contents at rest. They are access keys: whoever
//! has them can use the subscription. On Windows they are sealed with DPAPI (only this
//! Windows account can open them), on macOS with a key kept in the Keychain. On Linux and
//! Android they stay as files only their owner can read: a key would have to sit next to
//! them, and Android keeps each app's files apart already.
//!
//! Sealed text is stored as `kova-sealed:v1:<base64>`; anything else is read as it is,
//! so files from before this change still open, and are sealed on their next save.

const PREFIX: &str = "kova-sealed:v1:";

/// `text` sealed for storage, or as it is where sealing isn't available or fails.
pub fn seal(text: &str) -> String {
    match imp::seal(text.as_bytes()) {
        Some(b) => format!("{PREFIX}{}", b64::encode(&b)),
        None => text.to_string(),
    }
}

/// The text `stored` holds. `None` when it is sealed but can't be opened here (copied
/// from another computer or account).
pub fn open(stored: &str) -> Option<String> {
    let Some(rest) = stored.strip_prefix(PREFIX) else { return Some(stored.to_string()) };
    let data = b64::decode(rest)?;
    imp::open(&data).and_then(|b| String::from_utf8(b).ok())
}

/// Whether this system seals at all (Windows, macOS).
pub fn available() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

pub fn is_sealed(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

mod b64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    pub fn encode(b: &[u8]) -> String {
        STANDARD.encode(b)
    }
    pub fn decode(s: &str) -> Option<Vec<u8>> {
        STANDARD.decode(s.trim()).ok()
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};

    fn call(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: u32::try_from(data.len()).ok()?, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: `input` points at `data` for the duration of the call; DPAPI allocates
        // `out` with LocalAlloc, copied out and freed below.
        let ok = unsafe {
            if protect {
                CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            } else {
                CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            }
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        // SAFETY: DPAPI returned `cbData` bytes at `pbData`.
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: allocated by DPAPI with LocalAlloc.
        unsafe { LocalFree(out.pbData.cast()) };
        Some(v)
    }

    pub fn seal(data: &[u8]) -> Option<Vec<u8>> {
        call(data, true)
    }
    pub fn open(data: &[u8]) -> Option<Vec<u8>> {
        call(data, false)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
    use security_framework::passwords::{get_generic_password, set_generic_password};

    const SERVICE: &str = "app.kova.client";
    const ACCOUNT: &str = "profiles key";

    /// The key in the Keychain, made the first time.
    fn key() -> Option<ChaCha20Poly1305> {
        let k = match get_generic_password(SERVICE, ACCOUNT) {
            Ok(k) if k.len() == 32 => k,
            _ => {
                let mut k = vec![0u8; 32];
                getrandom::fill(&mut k).ok()?;
                set_generic_password(SERVICE, ACCOUNT, &k).ok()?;
                k
            }
        };
        Some(ChaCha20Poly1305::new(Key::from_slice(&k)))
    }

    pub fn seal(data: &[u8]) -> Option<Vec<u8>> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).ok()?;
        let mut out = nonce.to_vec();
        out.extend(key()?.encrypt(Nonce::from_slice(&nonce), data).ok()?);
        Some(out)
    }
    pub fn open(data: &[u8]) -> Option<Vec<u8>> {
        if data.len() < 12 {
            return None;
        }
        let (nonce, sealed) = data.split_at(12);
        key()?.decrypt(Nonce::from_slice(nonce), sealed).ok()
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn seal(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
    pub fn open(_: &[u8]) -> Option<Vec<u8>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_text_opens_and_plain_text_passes_through() {
        let url = "https://sub.example.com/T0kenForTests123";
        let stored = seal(url);
        if cfg!(any(windows, target_os = "macos")) {
            assert!(is_sealed(&stored), "{stored}");
            assert!(!stored.contains("T0kenForTests123"));
        }
        assert_eq!(open(&stored).as_deref(), Some(url));
        assert_eq!(open(url).as_deref(), Some(url), "files from before sealing still open");
        assert_eq!(open("kova-sealed:v1:bm90IHNlYWxlZA=="), None, "garbage doesn't pass as a link");
    }
}
