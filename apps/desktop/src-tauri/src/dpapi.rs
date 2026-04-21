//! Thin wrapper over Windows DPAPI (`CryptProtectData` / `CryptUnprotectData`).
//!
//! DPAPI encrypts data using a key derived from the current Windows user's
//! login secret. The ciphertext is only decryptable by the same user on the
//! same machine — which is exactly what we want for credentials-at-rest:
//! another local user account can't read a NanoCrew drive's S3 secret, and
//! the SQLite file is useless when copied to a different machine.
//!
//! We pass `CRYPTPROTECT_UI_FORBIDDEN` so DPAPI never pops a UI prompt, even
//! on profiles where a smart card is associated.

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
};

use crate::error::AppError;

/// Encrypt `plaintext` with the current user's DPAPI key.
pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };

    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &input,
            None,                         // szDataDescr
            None,                         // pOptionalEntropy
            None,                         // pvReserved
            None,                         // pPromptStruct
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| AppError::Keyring(format!("DPAPI protect failed: {e}")))?;
    }

    let bytes = unsafe {
        std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
    };
    unsafe {
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
    }

    Ok(bytes)
}

/// Reverse of [`protect`]. Fails if this user/machine cannot decrypt the blob
/// (e.g. when it was created under a different user account or copied from
/// another machine).
pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };

    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(
            &input,
            None,                         // ppszDataDescr
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| AppError::Keyring(format!("DPAPI unprotect failed: {e}")))?;
    }

    let bytes = unsafe {
        std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
    };
    unsafe {
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
    }

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let pt = b"hunter2-very-secret-key";
        let ct = protect(pt).expect("protect");
        assert_ne!(ct, pt, "ciphertext must differ from plaintext");
        let rt = unprotect(&ct).expect("unprotect");
        assert_eq!(rt, pt);
    }

    #[test]
    fn tampered_blob_fails() {
        let ct = protect(b"abc").unwrap();
        let mut bad = ct.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0xFF;
        assert!(unprotect(&bad).is_err());
    }
}
