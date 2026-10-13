//! Read-only decryption for the verified Windows Claude Desktop storage format.
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};

const MAX_BYTES: usize = 1024 * 1024;
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const INVALID: &str =
    "Claude Desktop login could not be decrypted. Open Claude Desktop, then Refresh.";
const UNSUPPORTED: &str =
    "This Claude Desktop login encryption format is not supported by this build.";

pub(super) fn decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext.len() > MAX_BYTES || key.len() != KEY_BYTES {
        return Err(INVALID.into());
    }
    if !ciphertext.starts_with(b"v10") {
        return Err(UNSUPPORTED.into());
    }
    if ciphertext.len() < 3 + NONCE_BYTES + TAG_BYTES {
        return Err(INVALID.into());
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| INVALID.to_owned())?;
    let nonce = Nonce::from_slice(&ciphertext[3..3 + NONCE_BYTES]);
    let plaintext = cipher
        .decrypt(nonce, &ciphertext[3 + NONCE_BYTES..])
        .map_err(|_| INVALID.to_owned())?;
    if plaintext.len() > MAX_BYTES {
        return Err(INVALID.into());
    }
    Ok(plaintext)
}

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB},
};

// CryptUnprotectData allocates its output. Release it even on validation failures.
#[cfg(windows)]
#[derive(Default)]
struct LocalBlob(CRYPT_INTEGER_BLOB);

#[cfg(windows)]
impl Drop for LocalBlob {
    fn drop(&mut self) {
        if !self.0.pbData.is_null() {
            // Remove plaintext from the native allocation before returning it to Windows.
            // cbData comes from Windows, and input/output bounds prevent unbounded clearing.
            for offset in 0..(self.0.cbData as usize).min(MAX_BYTES) {
                unsafe { std::ptr::write_volatile(self.0.pbData.add(offset), 0) };
            }
            unsafe { LocalFree(self.0.pbData.cast()) };
        }
    }
}

#[cfg(windows)]
pub(super) fn unprotect_key(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    if encrypted.len() > MAX_BYTES {
        return Err(INVALID.into());
    }
    let protected = encrypted.strip_prefix(b"DPAPI").ok_or(UNSUPPORTED)?;
    if protected.is_empty() {
        return Err(INVALID.into());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr().cast_mut(),
    };
    let mut output = LocalBlob::default();
    let success = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.0,
        )
    };
    if success == 0 || output.0.cbData as usize != KEY_BYTES || output.0.pbData.is_null() {
        return Err(INVALID.into());
    }
    Ok(unsafe { std::slice::from_raw_parts(output.0.pbData, KEY_BYTES) }.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encrypt(plaintext: &[u8], key: &[u8]) -> Vec<u8> {
        let cipher = Aes256Gcm::new_from_slice(key).unwrap();
        let nonce = [7; NONCE_BYTES];
        let mut encrypted = b"v10".to_vec();
        encrypted.extend_from_slice(&nonce);
        encrypted.extend(
            cipher
                .encrypt(Nonce::from_slice(&nonce), plaintext)
                .unwrap(),
        );
        encrypted
    }

    #[test]
    fn authenticated_round_trip_and_empty_plaintext() {
        let key = [42; KEY_BYTES];
        for plaintext in [br#"{"accessToken":"synthetic-only"}"#.as_slice(), b""] {
            assert_eq!(decrypt(&encrypt(plaintext, &key), &key).unwrap(), plaintext);
        }
    }

    #[test]
    fn rejects_tampered_nonce_ciphertext_tag_and_wrong_key_with_safe_error() {
        let key = [42; KEY_BYTES];
        let ciphertext = encrypt(b"private synthetic test payload", &key);
        for position in [3, 3 + NONCE_BYTES, ciphertext.len() - 1] {
            let mut changed = ciphertext.clone();
            changed[position] ^= 1;
            assert_eq!(decrypt(&changed, &key).unwrap_err(), INVALID);
        }
        assert_eq!(decrypt(&ciphertext, &[43; KEY_BYTES]).unwrap_err(), INVALID);
    }

    #[test]
    fn rejects_unsupported_wrappers_malformed_inputs_and_wrong_key_size() {
        let key = [42; KEY_BYTES];
        for wrapper in [b"v11".as_slice(), b"v20", b"DPAPI", b"", b"plain secret"] {
            assert_eq!(decrypt(wrapper, &key).unwrap_err(), UNSUPPORTED);
        }
        for ciphertext in [b"v10".to_vec(), vec![0; 3 + NONCE_BYTES + TAG_BYTES - 1]] {
            let mut ciphertext = ciphertext;
            ciphertext[..3].copy_from_slice(b"v10");
            assert_eq!(decrypt(&ciphertext, &key).unwrap_err(), INVALID);
        }
        let ciphertext = encrypt(b"synthetic", &key);
        for length in [0, 16, 31, 33, 64] {
            assert_eq!(
                decrypt(&ciphertext, &vec![42; length]).unwrap_err(),
                INVALID
            );
        }
    }

    #[test]
    fn enforces_ciphertext_limit_at_the_boundary() {
        let key = [42; KEY_BYTES];
        let plaintext = vec![0; MAX_BYTES - 3 - NONCE_BYTES - TAG_BYTES];
        let mut ciphertext = encrypt(&plaintext, &key);
        assert_eq!(ciphertext.len(), MAX_BYTES);
        assert_eq!(decrypt(&ciphertext, &key).unwrap(), plaintext);
        ciphertext.push(0);
        assert_eq!(decrypt(&ciphertext, &key).unwrap_err(), INVALID);
    }

    #[cfg(windows)]
    fn protect_key(plaintext: &[u8]) -> Vec<u8> {
        use windows_sys::Win32::Security::Cryptography::CryptProtectData;
        let input = CRYPT_INTEGER_BLOB {
            cbData: plaintext.len() as u32,
            pbData: plaintext.as_ptr().cast_mut(),
        };
        let mut output = LocalBlob::default();
        let success = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        };
        assert_ne!(success, 0);
        let mut wrapped = b"DPAPI".to_vec();
        wrapped.extend(unsafe {
            std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize)
        });
        wrapped
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_synthetic_round_trip_and_rejected_master_keys() {
        let key = [42; KEY_BYTES];
        let encrypted = protect_key(&key);
        assert_eq!(unprotect_key(&encrypted).unwrap(), key);
        for length in [0, 16, 31, 33, 64] {
            assert_eq!(
                unprotect_key(&protect_key(&vec![42; length])).unwrap_err(),
                INVALID
            );
        }
        assert_eq!(unprotect_key(b"DPAPI").unwrap_err(), INVALID);
        assert_eq!(unprotect_key(b"DPAPIinvalid").unwrap_err(), INVALID);
        assert_eq!(unprotect_key(b"unknown").unwrap_err(), UNSUPPORTED);
        assert_eq!(unprotect_key(&vec![0; MAX_BYTES + 1]).unwrap_err(), INVALID);
    }
}
