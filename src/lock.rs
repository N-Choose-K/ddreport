//! Marks a finished PDF read-only: AES-128 (PDF 1.6) with an empty user
//! password so it opens anywhere, editing permissions withheld, and a random
//! owner password that is never stored. Compliant readers (Preview, Acrobat,
//! browsers) refuse edits; a PDF library can still strip the restrictions, so
//! this guards against casual changes only. Reports are regenerated, never
//! edited. AES-128 because poppler cannot read lopdf 0.46's AES-256 output.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use lopdf::Document;
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::encryption::{EncryptionState, EncryptionVersion, Permissions};
use rand::RngCore;

/// Everything a reader may do with the file; editing is excluded.
pub const ALLOWED: Permissions = Permissions::PRINTABLE
    .union(Permissions::PRINTABLE_IN_HIGH_QUALITY)
    .union(Permissions::COPYABLE)
    .union(Permissions::COPYABLE_FOR_ACCESSIBILITY);

/// Encrypts `pdf` so it opens without a password and compliant readers
/// refuse to edit it.
pub fn lock(pdf: &[u8]) -> Result<Vec<u8>> {
    let mut doc = Document::load_mem(pdf).context("parsing the rendered PDF")?;
    let mut owner = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut owner);
    let owner: String = owner.iter().map(|b| format!("{b:02x}")).collect();

    let filters: BTreeMap<Vec<u8>, Arc<dyn CryptFilter>> =
        BTreeMap::from([(b"StdCF".to_vec(), Arc::new(Aes128CryptFilter) as _)]);
    let state = EncryptionState::try_from(EncryptionVersion::V4 {
        document: &doc,
        encrypt_metadata: true,
        crypt_filters: filters,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: &owner,
        user_password: "",
        permissions: ALLOWED,
    })
    .context("building PDF encryption parameters")?;
    doc.encrypt(&state).context("encrypting the PDF")?;

    let mut out = Vec::with_capacity(pdf.len() + 1024);
    doc.save_to(&mut out).context("writing the locked PDF")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let r = crate::model::DriveReport::default();
        crate::pdf::render_unlocked(&r, &crate::pdf::PdfOptions::default()).unwrap()
    }

    #[test]
    fn opens_without_a_password_and_forbids_editing() {
        let locked = lock(&sample()).unwrap();
        // Loading decrypts with the empty user password on its own.
        let doc = Document::load_mem(&locked).unwrap();
        assert!(doc.was_encrypted());
        assert!(doc.authenticate_owner_password("").is_err());
        let state = doc.encryption_state.as_ref().unwrap();
        assert_eq!(state.version(), 4);
        let p = state.permissions();
        assert!(p.contains(Permissions::PRINTABLE));
        assert!(!p.contains(Permissions::MODIFIABLE));
        assert!(!p.contains(Permissions::ANNOTABLE));
        assert!(!p.contains(Permissions::FILLABLE));
        assert!(!p.contains(Permissions::ASSEMBLABLE));
    }

    #[test]
    fn content_survives_locking() {
        let plain = sample();
        let locked = lock(&plain).unwrap();
        let doc = Document::load_mem(&locked).unwrap();
        let text = doc.extract_text(&[1]).unwrap();
        assert!(text.contains("Drive Health Certificate"), "{text}");
    }

    #[test]
    fn owner_password_differs_every_time() {
        let plain = sample();
        assert_ne!(lock(&plain).unwrap(), lock(&plain).unwrap());
    }
}
