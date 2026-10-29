//! LUKSbox's native WebAuthn (Windows) / libfido2 (Linux) transport.
#[cfg(target_os = "windows")]
use crate::windows_passkey::WindowsAuthenticator as NativeAuthenticator;
use anyhow::{Context, Result, ensure};
use luksbox_core::{Argon2idParams, Header, SlotKind, header::HEADER_SIZE};
#[cfg(not(target_os = "windows"))]
use luksbox_fido2::HidAuthenticator as NativeAuthenticator;
use luksbox_fido2::{Fido2Authenticator, HmacSecretRequest, RP_ID};
use luksbox_format::{Container, UnlockMaterial};
use rand_core::{OsRng, RngCore};
use std::{io::Read, path::Path};

pub(crate) fn authenticator(hello: bool) -> Result<NativeAuthenticator> {
    ensure!(
        std::env::var_os("LUKSBOX_FIDO2_DEBUG").is_none(),
        "Remove LUKSBOX_FIDO2_DEBUG; raw authenticator diagnostics are not allowed"
    );
    #[cfg(target_os = "windows")]
    {
        Ok(NativeAuthenticator::new(hello)?)
    }
    #[cfg(not(target_os = "windows"))]
    {
        ensure!(!hello, "Windows Hello is only available on Windows");
        Ok(NativeAuthenticator::new())
    }
}

pub(crate) fn enroll(
    container: &mut Container,
    auth: &mut dyn Fido2Authenticator,
    pin: Option<&str>,
) -> Result<usize> {
    ensure!(
        container.populated_slot_count() < 8,
        "All keyslots are occupied"
    );
    let mut user = [0u8; 32];
    let mut salt = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut user)
        .map_err(|_| anyhow::anyhow!("Cannot generate credential user handle"))?;
    OsRng
        .try_fill_bytes(&mut salt)
        .map_err(|_| anyhow::anyhow!("Cannot generate Passkey salt"))?;
    let credential = auth
        .enroll(RP_ID, &user, pin)
        .context("Passkey registration failed")?
        .credential;
    // Registration alone is insufficient: prove the authenticator actually supplies
    // stable PRF material before adding a way to unwrap the volume key.
    let first = auth
        .hmac_secret(RP_ID, &credential.id, &salt, true, pin)
        .context("Passkey first PRF verification failed")?;
    ensure!(
        first.iter().any(|byte| *byte != 0),
        "Invalid Passkey secret"
    );
    let verified = auth
        .hmac_secret(RP_ID, &credential.id, &salt, true, pin)
        .context("Passkey second PRF verification failed")?;
    ensure!(
        first == verified,
        "Passkey secret is not stable; keyslot was not added"
    );
    Ok(container.enroll_fido2(
        None,
        &first,
        &credential.id,
        salt,
        Argon2idParams::INTERACTIVE,
    )?)
}

pub(crate) fn open(
    path: &Path,
    auth: &mut dyn Fido2Authenticator,
    pin: Option<&str>,
) -> Result<Container> {
    let mut file = luksbox_core::file_util::open_existing_read_no_follow_policy(path)?;
    let mut bytes = [0u8; HEADER_SIZE];
    file.read_exact(&mut bytes)?;
    drop(file);
    let header = Header::from_bytes(&bytes).context("Invalid LUKSbox header")?;
    let slots: Vec<_> = header
        .keyslots
        .iter()
        .filter(|slot| slot.kind == SlotKind::Fido2HmacSecret)
        .collect();
    ensure!(!slots.is_empty(), "No Passkey keyslot in this vault");
    let requests: Vec<_> = slots
        .iter()
        .map(|slot| HmacSecretRequest {
            cred_id: &slot.fido2_cred_id,
            salt: &slot.fido2_hmac_salt,
            prehash_salt: slot.fido2_salt_prehashed(),
        })
        .collect();
    let (index, secret) = auth
        .hmac_secret_multi(RP_ID, &requests, pin)
        .context("Passkey unlock verification failed")?;
    let slot = slots.get(index).context("Invalid authenticator response")?;
    Container::open(
        path,
        None,
        UnlockMaterial::Fido2 {
            cred_id: &slot.fido2_cred_id,
            passphrase: None,
            hmac_secret: &secret,
        },
    )
    .context("Passkey cannot unlock this vault")
}
