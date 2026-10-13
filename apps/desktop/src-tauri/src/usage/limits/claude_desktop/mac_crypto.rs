//! Electron macOS v10: PBKDF2-HMAC-SHA1 + AES-128-CBC with PKCS7 padding.
//! CBC does not authenticate the payload. The loader also validates JSON/account/cookie metadata.
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use zeroize::Zeroizing;

const MAX_BYTES: usize = 1024 * 1024;
const INVALID: &str =
    "Claude Desktop login could not be decrypted. Open Claude Desktop, then Refresh.";

pub(super) fn derive_key(password: &[u8]) -> Result<Zeroizing<[u8; 16]>, String> {
    if password.is_empty() || password.len() > 4096 {
        return Err(INVALID.into());
    }
    let mut key = Zeroizing::new([0; 16]);
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, key.as_mut());
    Ok(key)
}

pub(super) fn decrypt(ciphertext: &[u8], key: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext.len() > MAX_BYTES || key.len() != 16 {
        return Err(INVALID.into());
    }
    let payload = ciphertext
        .strip_prefix(b"v10")
        .ok_or("This Claude Desktop login encryption format is not supported by this build.")?;
    if payload.is_empty() || payload.len() % 16 != 0 {
        return Err(INVALID.into());
    }
    // Clear a failed decode too; the buffer may already contain partial plaintext.
    let mut clear = Zeroizing::new(payload.to_vec());
    let decrypted = cbc::Decryptor::<aes::Aes128>::new_from_slices(key, &[b' '; 16])
        .map_err(|_| INVALID.to_owned())?
        .decrypt_padded_mut::<Pkcs7>(&mut clear)
        .map_err(|_| INVALID.to_owned())?;
    Ok(decrypted.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncryptMut;
    use base64::{engine::general_purpose::STANDARD, Engine};

    pub(super) fn encrypt(plain: &[u8], key: &[u8]) -> Vec<u8> {
        let mut value = b"v10".to_vec();
        value.extend(
            cbc::Encryptor::<aes::Aes128>::new_from_slices(key, &[b' '; 16])
                .unwrap()
                .encrypt_padded_vec_mut::<Pkcs7>(plain),
        );
        value
    }

    #[test]
    fn independent_electron_cbc_vector_matches_pbkdf_and_decryption() {
        // Generated independently with Python hashlib and system OpenSSL, using synthetic secrets.
        let key = derive_key(b"fixture-safe-storage-password").unwrap();
        assert_eq!(
            key.as_slice(),
            &[
                0xae, 0x1a, 0x2f, 0x63, 0xe3, 0xbb, 0x06, 0xfc, 0x8c, 0x04, 0xa1, 0x29, 0x08, 0x01,
                0xac, 0x5d
            ]
        );
        let encrypted = STANDARD
            .decode("djEwLU4uBqFaI1hV2oj0BGOl449UMWPZV/aawhNj8jXTZ00=")
            .unwrap();
        assert_eq!(
            decrypt(&encrypted, key.as_slice()).unwrap(),
            br#"{"token":"synthetic-access"}"#
        );
    }

    #[test]
    fn mac_format_rejects_wrappers_key_sizes_padding_and_unbounded_data() {
        let key = [42; 16];
        assert_eq!(decrypt(&encrypt(b"", &key), &key).unwrap(), b"");
        for value in [b"v10".as_slice(), b"v10short", b"v11", b"v20"] {
            assert!(decrypt(value, &key).is_err());
        }
        for size in [0, 15, 17, 32] {
            assert!(decrypt(&encrypt(b"synthetic", &key), &vec![0; size]).is_err());
        }
        let mut invalid = encrypt(b"synthetic", &key);
        *invalid.last_mut().unwrap() ^= 0x80;
        assert_eq!(decrypt(&invalid, &key).unwrap_err(), INVALID);
        assert_eq!(decrypt(&vec![0; MAX_BYTES + 1], &key).unwrap_err(), INVALID);
        assert!(derive_key(b"").is_err());
        assert!(derive_key(&vec![0; 4097]).is_err());
    }
}
